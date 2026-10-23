use orr_asset::{
    encode_manifest, manifest_encoded_len, AssetRef, Domain, ManifestEntry, IMPACT_PCM16_TYPE_ID,
};
use orr_asset_cook::{hex, sha256, CookError, Result};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_LICENSE_BYTES: usize = 64 * 1024;

pub struct Options {
    pub wav: PathBuf,
    pub id: AssetRef,
    pub name: String,
    pub version: String,
    pub license: PathBuf,
    pub out: PathBuf,
}

#[derive(Serialize)]
struct Package<'a> {
    schema: u32,
    name: &'a str,
    version: &'a str,
    engine: &'static str,
    capabilities: [&'static str; 1],
    dependencies: BTreeMap<String, String>,
    files: Vec<String>,
}

fn validate_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

// SemVer 2.0.0 canonical spelling, matching the package manager's semver crate.
// No dependency is added merely for this deliberately small example.
fn validate_version(value: &str) -> bool {
    if value.is_empty() || value.len() > 256 {
        return false;
    }
    let (without_build, build) = value
        .split_once('+')
        .map_or((value, None), |(a, b)| (a, Some(b)));
    let (core, pre) = without_build
        .split_once('-')
        .map_or((without_build, None), |(a, b)| (a, Some(b)));
    let numbers: Vec<_> = core.split('.').collect();
    if numbers.len() != 3
        || !numbers
            .iter()
            .all(|s| numeric(s) && s.parse::<u64>().is_ok())
    {
        return false;
    }
    let identifiers = |s: &str, prerelease: bool| {
        s.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && (!prerelease || !part.bytes().all(|b| b.is_ascii_digit()) || numeric(part))
        })
    };
    pre.is_none_or(|s| identifiers(s, true)) && build.is_none_or(|s| identifiers(s, false))
}
fn numeric(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s.len() == 1 || !s.starts_with('0'))
}

// Reject static symlink traversal, special files and lexical parent traversal.
// These checks are not race-proof openat/no-follow operations: all directories
// must remain trusted and unchanged by other processes for the entire command.
fn checked_path(path: &Path, allow_missing_leaf: bool) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(CookError::invalid("empty path"));
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let parts: Vec<_> = absolute.components().collect();
    let mut checked = PathBuf::new();
    for (index, part) in parts.iter().enumerate() {
        if matches!(part, Component::ParentDir) {
            return Err(CookError::invalid("parent traversal is not allowed"));
        }
        checked.push(part.as_os_str());
        match fs::symlink_metadata(&checked) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(CookError::invalid("symlink paths are not allowed"));
                }
                if index + 1 < parts.len() && !meta.is_dir() {
                    return Err(CookError::invalid("path ancestor is not a directory"));
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && allow_missing_leaf
                    && index + 1 == parts.len() => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(checked)
}

fn read_regular(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let path = checked_path(path, false)?;
    let meta = fs::symlink_metadata(&path)?;
    if !meta.is_file() || meta.len() > limit as u64 {
        return Err(CookError::invalid("input must be a bounded regular file"));
    }
    let mut file = File::open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > limit as u64 {
        return Err(CookError::invalid("input changed or exceeds byte budget"));
    }
    let mut bytes = Vec::new();
    (&mut file).take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(CookError::invalid("input exceeds byte budget"));
    }
    Ok(bytes)
}

pub fn create(options: &Options) -> Result<String> {
    if options.id.is_null() {
        return Err(CookError::invalid(
            "asset ID must be nonzero and explicitly assigned",
        ));
    }
    if !validate_name(&options.name) {
        return Err(CookError::invalid("invalid lowercase package name"));
    }
    if !validate_version(&options.version) {
        return Err(CookError::invalid(
            "canonical semantic version required (maximum 256 bytes)",
        ));
    }
    let out = checked_path(&options.out, true)?;
    match fs::symlink_metadata(&out) {
        Ok(_) => {
            return Err(CookError::invalid(
                "output already exists; choose a new directory",
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let wav = read_regular(&options.wav, super::wav::MAX_WAV_BYTES)?;
    let license = read_regular(&options.license, MAX_LICENSE_BYTES)?;
    if license.is_empty() || std::str::from_utf8(&license).is_err() {
        return Err(CookError::invalid("license must be nonempty UTF-8 text"));
    }
    let payload = super::wav::parse_wav(&wav)?;
    orr_asset_cook::authoring::validate_payload(
        orr_asset_cook::authoring::AssetType::Impact,
        &payload,
    )?;
    let digest = sha256(&payload);
    let object = format!("cooked/objects/{}.bin", hex(&digest));
    let entry = ManifestEntry {
        id: options.id,
        type_id: IMPACT_PCM16_TYPE_ID,
        schema_version: 1,
        payload_len: payload.len() as u64,
        payload_sha256: digest,
    };
    let mut view = vec![0; manifest_encoded_len(Domain::View, 1)?];
    encode_manifest(Domain::View, &[entry], &mut view)?;
    let package = Package {
        schema: 1,
        name: &options.name,
        version: &options.version,
        engine: "^0.0.1",
        capabilities: ["collect-audio"],
        dependencies: BTreeMap::new(),
        files: vec![
            "LICENSE.txt".into(),
            object.clone(),
            "cooked/view.manifest.bin".into(),
        ],
    };
    let mut metadata = serde_json::to_vec_pretty(&package)?;
    metadata.push(b'\n');
    let files = [
        ("LICENSE.txt".to_owned(), license),
        ("cooked/view.manifest.bin".to_owned(), view),
        (object, payload),
        ("orr.package.json".to_owned(), metadata),
    ];
    publish(&out, &files)?;
    Ok(hex(&digest))
}

// Checked publication: mkdir claims a new output without replacing anything.
// It becomes visible before completion. A reported error removes only this
// command's claimed directory under the trusted-directory assumption above.
fn publish(out: &Path, files: &[(String, Vec<u8>)]) -> Result<()> {
    fs::create_dir(out)?;
    let result = (|| -> Result<()> {
        fs::create_dir(out.join("cooked"))?;
        fs::create_dir(out.join("cooked/objects"))?;
        for (name, bytes) in files {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(out.join(name))?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        if let Err(cleanup) = fs::remove_dir_all(out) {
            return Err(CookError::invalid(format!(
                "publication failed: {error}; cleanup also failed: {cleanup}; inspect {}",
                out.display()
            )));
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
