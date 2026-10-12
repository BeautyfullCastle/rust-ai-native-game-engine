//! Bounded immutable snapshots. Caller-selected paths and their ancestry must
//! remain trusted during reads; std path checks are not a hostile-race sandbox.
use orr_fp::FP;
use orr_terrain::Terrain;
use serde::Deserialize;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub const MAX_COURSE_BYTES: usize = 4096;
pub const MAX_TERRAIN_BYTES: usize = 8192;
pub const DEFAULT_COURSE: &[u8] = include_bytes!("../assets/hill_course.json");
pub const DEFAULT_TERRAIN: &[u8] = include_bytes!("../assets/hill_course.orrt");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Description {
    version: u32,
    title: String,
    terrain: String,
    spawn: [String; 2],
    beacons: [[String; 2]; 4],
    finish: [String; 2],
    radius: String,
}

#[derive(Clone, Debug)]
pub struct Course {
    terrain: Terrain,
    terrain_revision: [u8; 32],
    title: String,
    spawn: [FP; 3],
    beacons: [[FP; 3]; 4],
    finish: [FP; 3],
    radius: FP,
}

fn description(bytes: &[u8]) -> Result<Description, String> {
    if bytes.len() > MAX_COURSE_BYTES {
        return Err("course exceeds 4096 bytes".into());
    }
    let d: Description = serde_json::from_slice(bytes).map_err(|e| format!("course JSON: {e}"))?;
    if d.version != 1 {
        return Err("unsupported course version".into());
    }
    if d.title.is_empty()
        || d.title.len() > 48
        || !d.title.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
    {
        return Err("title must contain 1..48 printable ASCII bytes".into());
    }
    if d.terrain.len() > 64
        || !d.terrain.ends_with(".orrt")
        || d.terrain.starts_with('.')
        || !d
            .terrain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err("terrain must be a simple sibling .orrt filename".into());
    }
    Ok(d)
}

fn fixed(s: &str) -> Result<FP, String> {
    if s.len() > 32 {
        return Err("fixed-point text exceeds 32 bytes".into());
    }
    FP::parse(s).map_err(|e| format!("invalid fixed-point decimal: {e:?}"))
}

impl Course {
    pub fn builtin() -> Result<Self, String> {
        Self::from_bytes(DEFAULT_COURSE, DEFAULT_TERRAIN)
    }

    /// Parse owned canonical terrain and strict course description snapshots.
    /// Decimal strings are rounded by FP::parse, never through a float.
    pub fn from_bytes(json: &[u8], bytes: &[u8]) -> Result<Self, String> {
        let d = description(json)?;
        if bytes.len() > MAX_TERRAIN_BYTES {
            return Err("terrain exceeds 8192 bytes".into());
        }
        let terrain = Terrain::load(bytes).map_err(|e| format!("terrain: {e}"))?;
        if terrain.width() > 17 || terrain.depth() > 17 {
            return Err("course terrain dimensions must be 2..17".into());
        }
        if !(16384..=524288).contains(&terrain.spacing().raw()) {
            return Err("course terrain spacing must be 1/4..8".into());
        }
        // In this bounded profile every Q16 coordinate is exactly f32-displayable.
        let bound = 128 * 65536;
        for i in 0..terrain.width() * terrain.depth() {
            if terrain
                .vertex_position(i)
                .expect("validated terrain")
                .iter()
                .any(|v| !(-bound..=bound).contains(&v.raw()))
            {
                return Err("course terrain XYZ must be within +/-128".into());
            }
        }
        let anchor = |p: &[String; 2]| -> Result<[FP; 3], String> {
            let x = fixed(&p[0])?;
            let z = fixed(&p[1])?;
            let y = terrain
                .sample(x, z)
                .ok_or("course anchor is outside terrain or in a hole")?;
            Ok([x, y, z])
        };
        let spawn = anchor(&d.spawn)?;
        let mut beacons = [[FP::ZERO; 3]; 4];
        for (out, point) in beacons.iter_mut().zip(&d.beacons) {
            *out = anchor(point)?;
        }
        let finish = anchor(&d.finish)?;
        let radius = fixed(&d.radius)?;
        if !(16384..=65536).contains(&radius.raw()) {
            return Err("checkpoint radius must be 1/4..1".into());
        }
        let terrain_revision = terrain.revision();
        Ok(Self {
            terrain,
            terrain_revision,
            title: d.title,
            spawn,
            beacons,
            finish,
            radius,
        })
    }

    /// Read each regular file once. Restart uses this snapshot, never rereads it.
    /// Parents must not be writable by an attacker during admission (see docs).
    pub fn load(path: &Path) -> Result<Self, String> {
        let path = checked_absolute(path)?;
        let json = read_regular(&path, MAX_COURSE_BYTES)?;
        let d = description(&json)?;
        let terrain_path = path
            .parent()
            .ok_or("course path has no parent")?
            .join(&d.terrain);
        let bytes = read_regular(&terrain_path, MAX_TERRAIN_BYTES)?;
        Self::from_bytes(&json, &bytes)
    }

    pub fn terrain(&self) -> &Terrain {
        &self.terrain
    }
    pub fn terrain_revision(&self) -> [u8; 32] {
        self.terrain_revision
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn spawn(&self) -> [FP; 3] {
        self.spawn
    }
    pub fn beacons(&self) -> &[[FP; 3]; 4] {
        &self.beacons
    }
    pub fn finish(&self) -> [FP; 3] {
        self.finish
    }
    pub fn radius(&self) -> FP {
        self.radius
    }
}

fn checked_absolute(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() || path.components().any(|c| c == Component::ParentDir) {
        return Err("empty paths and parent traversal are not accepted".into());
    }
    // On Windows, joining C:relative replaces the base with a drive-relative
    // path. Refuse it before joining rather than inspecting the wrong ancestry.
    if !path.is_absolute() && path.components().any(|c| matches!(c, Component::Prefix(_))) {
        return Err("prefixed paths must be absolute".into());
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    if !absolute.is_absolute() {
        return Err("input path did not resolve to an absolute path".into());
    }
    Ok(absolute)
}

fn read_regular(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component.as_os_str());
        // A Windows verbatim disk prefix (\\?\C:) is incomplete until its
        // RootDir arrives. Check the assembled root and every real ancestor.
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let metadata =
            fs::symlink_metadata(&prefix).map_err(|e| format!("{}: {e}", prefix.display()))?;
        if metadata.file_type().is_symlink() {
            return Err("symlink input or ancestor is not accepted".into());
        }
    }
    let before = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !before.is_file() || before.len() > limit as u64 {
        return Err("input must be a bounded regular file".into());
    }
    // Precheck avoids opening a static FIFO/device/socket. Hostile path swaps are
    // outside the trusted-ancestry contract; this is not an O_NOFOLLOW sandbox.
    let file = File::open(path).map_err(|e| e.to_string())?;
    let opened = file.metadata().map_err(|e| e.to_string())?;
    if !opened.is_file() || opened.len() > limit as u64 {
        return Err("opened input must be a bounded regular file".into());
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit || bytes.len() as u64 != opened.len() {
        return Err("input changed length or exceeded its byte budget".into());
    }
    Ok(bytes)
}
