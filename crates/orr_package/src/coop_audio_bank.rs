//! Optional fixed CoopDefense soundbank authoring. No device, install or playback.
use crate::authoring_io::{directory, error, read_at, recheck_directory};
use orr_asset_cook::authoring::{validate_payload, AssetType};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Component, Path, PathBuf},
};
mod publish;

pub use crate::coop_audio_admission::CUES;
const MAX_PCM: usize = 96008;
const MAX_LICENSE: usize = 64 * 1024;
#[derive(Clone, Copy, Debug)]
pub enum InputKind {
    Pcm,
    Wav,
}
#[derive(Debug)]
pub struct Options {
    pub cues: Vec<(String, PathBuf)>,
    pub license: PathBuf,
    pub output: PathBuf,
}
#[derive(Debug)]
pub struct Report {
    pub output: PathBuf,
    pub durability_warning: Option<String>,
}
#[derive(Serialize)]
struct Manifest {
    version: u32,
    scope: &'static str,
    #[serde(rename = "sampleRate")]
    sample_rate: u32,
    cues: Vec<Record>,
}
#[derive(Serialize)]
struct Record {
    cue: &'static str,
    #[serde(rename = "assetId")]
    asset_id: String,
    file: String,
    frames: u32,
    bytes: usize,
    sha256: String,
}
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("empty path".into());
    }
    let p = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().map_err(error)?.join(path)
    };
    if p.as_os_str().as_encoded_bytes().len() > 4096
        || p.components().count() > 128
        || p.components().any(|c| matches!(c, Component::ParentDir))
        || p.as_os_str().as_encoded_bytes().contains(&0)
    {
        return Err("path exceeds bounds or contains traversal/NUL".into());
    }
    Ok(p)
}
fn snapshot(path: &Path, max: usize) -> Result<Vec<u8>, String> {
    let path = absolute(path)?;
    let parent_path = path.parent().ok_or("missing input parent")?;
    let parent = directory(parent_path)?;
    let bytes = read_at(
        &parent,
        path.file_name().ok_or("missing input filename")?,
        max,
    )?;
    recheck_directory(parent_path, &parent)?;
    Ok(bytes)
}
impl Options {
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let mut args = Vec::new();
        let mut bytes = 0usize;
        for arg in arguments {
            bytes = bytes
                .checked_add(arg.as_bytes().len() + 1)
                .ok_or("argument overflow")?;
            if bytes > 64 * 1024 || args.len() >= 24 {
                return Err("arguments exceed fixed bank bounds".into());
            }
            args.push(arg);
        }
        if args.len() != 24 {
            return Err("exactly ten --cue pairs, --license and --output are required".into());
        }
        let (mut cues, mut license, mut output) = (Vec::new(), None, None);
        for p in args.chunks_exact(2) {
            match p[0].to_str(){
            Some("--cue")=>{let raw=p[1].as_bytes();let split=raw.iter().position(|c|*c==b'=').ok_or("cue requires NAME=PATH")?;let name=std::str::from_utf8(&raw[..split]).map_err(|_|"cue name must be UTF-8")?.to_owned();let path=PathBuf::from(OsString::from_vec(raw[split+1..].to_vec()));cues.push((name,path));},
            Some("--license") if license.is_none()=>license=Some(PathBuf::from(&p[1])),
            Some("--output") if output.is_none()=>output=Some(PathBuf::from(&p[1])),
            _=>return Err("unknown, duplicate or obsolete option; external --importer/commands are not supported".into()),
        }
        }
        let options = Self {
            cues,
            license: license.ok_or("missing license")?,
            output: output.ok_or("missing output")?,
        };
        options.sources()?;
        Ok(options)
    }
    fn sources(&self) -> Result<BTreeMap<&str, &Path>, String> {
        if self.cues.len() != CUES.len() {
            return Err("exactly ten cues required".into());
        }
        let mut sources = BTreeMap::new();
        for (cue, path) in &self.cues {
            if !CUES.contains(&cue.as_str())
                || path.as_os_str().is_empty()
                || sources.insert(cue.as_str(), path.as_path()).is_some()
            {
                return Err("each supported cue must occur exactly once".into());
            }
        }
        Ok(sources)
    }
}
/// Snapshots every input before decode/publication. No child timeout is claimed.
pub fn create(options: &Options, kind: InputKind) -> Result<Report, String> {
    let sources = options.sources()?;
    if options.output.file_name().is_none() {
        return Err("output must name a new directory".into());
    }
    let output = absolute(&options.output)?;
    publish::preflight(&output)?;
    let license = snapshot(&options.license, MAX_LICENSE)?;
    if std::str::from_utf8(&license)
        .map_err(|_| "license must be UTF-8")?
        .trim_matches(|c: char| c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}'))
        .is_empty()
    {
        return Err("license must be nonempty UTF-8".into());
    }
    let limit = match kind {
        InputKind::Pcm => MAX_PCM,
        InputKind::Wav => orr_asset_cook::wav_import::MAX_WAV_BYTES,
    };
    let inputs = CUES
        .iter()
        .map(|cue| snapshot(sources[cue], limit))
        .collect::<Result<Vec<_>, _>>()?;
    let (mut files, mut records) = (Vec::new(), Vec::new());
    let (mut cooked, mut decoded) = (0usize, 0usize);
    for (index, (cue, input)) in CUES.into_iter().zip(inputs).enumerate() {
        let data = match kind {
            InputKind::Pcm => input,
            InputKind::Wav => orr_asset_cook::wav_import::parse_wav(&input).map_err(error)?,
        };
        validate_payload(AssetType::Impact, &data).map_err(error)?;
        if data.len() > MAX_PCM {
            return Err("PCM exceeds bank byte bound".into());
        }
        let frames = u32::from_le_bytes(data[4..8].try_into().unwrap());
        cooked += data.len();
        decoded += frames as usize * 8;
        if cooked > 2 * 1024 * 1024 || decoded > 4 * 1024 * 1024 {
            return Err("audio bank exceeds aggregate budget".into());
        }
        let file = format!("{cue}.pcm");
        records.push(Record {
            cue,
            asset_id: format!("a_000000005444{:04x}", index + 1),
            file: file.clone(),
            frames,
            bytes: data.len(),
            sha256: Sha256::digest(&data)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        });
        files.push((file, data));
    }
    let manifest = Manifest {
        version: 1,
        scope: "user-authored-coop-defense-bank",
        sample_rate: 48000,
        cues: records,
    };
    let mut json = serde_json::to_vec_pretty(&manifest).map_err(error)?;
    json.push(b'\n');
    if json.len() > 128 * 1024 {
        return Err("bank manifest exceeds bound".into());
    }
    files.push(("bank.json".into(), json));
    files.push(("LICENSE.txt".into(), license));
    publish::publish(&output, &files)
}
pub fn run_cli(arguments: impl IntoIterator<Item = OsString>, kind: InputKind) -> u8 {
    let mut args = arguments.into_iter().peekable();
    if args.peek().is_some_and(|a| a == "--help") {
        args.next();
        if args.next().is_none() {
            println!("orr_pkg create-coop-audio-bank[-from-wav] --cue NAME=PATH (exactly ten supported cues) --license FILE --output NEW_DIRECTORY\nLinux audio-authoring feature; fixed mono PCM16/48kHz/1..48000 frames/peak8192. No importer/shell/install/playback/rights verification. Visible partial publication; inspect failures, do not blindly retry. Cues: {}",CUES.join(", "));
            return 0;
        }
        return 2;
    }
    let options = match Options::parse(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Audio bank usage: {e}");
            return 2;
        }
    };
    match create(&options, kind) {
        Ok(report) => {
            println!("{}", report.output.display());
            if let Some(w) = report.durability_warning {
                eprintln!("warning: complete bank exists; durability uncertain: {w}; do not retry/overwrite/remove blindly");
            }
            0
        }
        Err(e) => {
            eprintln!("Audio bank creation failed: {e}");
            1
        }
    }
}
#[cfg(test)]
mod tests;
