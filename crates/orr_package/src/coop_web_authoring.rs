//! Optional web animation authoring. Python/Pillow-free; trusted Node remains required.
mod copy_assets;
mod manifest;
mod node_gate;
mod offline;
mod png;
mod prepared;
mod publish;
mod recipe;
mod sampler;
mod scene;
#[cfg(feature = "coop-native-authoring")]
mod native_scene;
use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::{Component, Path, PathBuf},
};
const USAGE:&str="orr_pkg create-coop-animation --assets-dir DIR --output NEW --role ROLE --image IMAGE --frame CLIP X Y W H [--frame ...] [--clip CLIP FPS loop|once] [--pivot X Y] [--attack-timing WINDUP_MS RELEASE_MS RECOVERY_MS]\norr_pkg compose-coop-animations --assets-dir DIR --output NEW --manifest FILE [--manifest FILE]...\nAttack timing requires attack frames; each duration is finite, 1..2000 ms.\nRequires trusted Node on PATH; no Python/Pillow. Final HTML packaging remains separate.";
fn absolute(path: &Path) -> Result<PathBuf, String> {
    let raw = path.as_os_str().as_encoded_bytes();
    if raw.is_empty()
        || raw.len() > 4096
        || raw.contains(&0)
        || path.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err("empty/oversized/traversal path".into());
    }
    let p = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    if p.components().count() > 128 || p.as_os_str().as_encoded_bytes().len() > 4096 {
        return Err("path exceeds bounds".into());
    }
    Ok(p)
}
fn text(value: &OsString) -> Result<String, String> {
    value
        .to_str()
        .filter(|s| s.len() <= 4096)
        .map(str::to_owned)
        .ok_or_else(|| "text must be bounded UTF-8".into())
}
fn number(value: &OsString) -> Result<f64, String> {
    text(value)?.parse().map_err(|_| "invalid number".into())
}
struct Options {
    assets: PathBuf,
    output: PathBuf,
    role: String,
    image: String,
    frames: Vec<(String, [f64; 4])>,
    clips: Vec<(String, f64, bool)>,
    pivot: Option<[f64; 2]>,
    attack_timing: Option<[f64; 3]>,
    manifests: Vec<PathBuf>,
}
fn options(args: &[OsString], compose: bool) -> Result<Options, String> {
    let mut assets = None;
    let mut output = None;
    let mut role = None;
    let mut image = None;
    let mut frames = Vec::new();
    let mut clips = Vec::new();
    let mut pivot = None;
    let mut attack_timing = None;
    let mut manifests = Vec::new();
    let mut seen = BTreeSet::new();
    let mut i = 0;
    while i < args.len() {
        let key = args[i].to_str().ok_or("flag must be Unicode")?;
        i += 1;
        let n = match key {
            "--assets-dir" | "--output" => 1,
            "--manifest" if compose => 1,
            "--role" | "--image" if !compose => 1,
            "--frame" if !compose => 5,
            "--clip" | "--attack-timing" if !compose => 3,
            "--pivot" if !compose => 2,
            _ => return Err("unknown animation flag".into()),
        };
        if i + n > args.len() {
            return Err("missing animation flag values".into());
        }
        if !matches!(key, "--frame" | "--clip" | "--manifest") && !seen.insert(key) {
            return Err("duplicate singleton flag".into());
        }
        let v = &args[i..i + n];
        i += n;
        match key {
            "--assets-dir" => assets = Some(PathBuf::from(&v[0])),
            "--output" => output = Some(PathBuf::from(&v[0])),
            "--role" => role = Some(text(&v[0])?),
            "--image" => image = Some(text(&v[0])?),
            "--frame" => {
                if frames.len() >= 1280 {
                    return Err("too many frames".into());
                }
                frames.push((
                    text(&v[0])?,
                    [
                        number(&v[1])?,
                        number(&v[2])?,
                        number(&v[3])?,
                        number(&v[4])?,
                    ],
                ));
            }
            "--clip" => {
                if clips.len() >= 5 {
                    return Err("too many clip settings".into());
                }
                let mode = match v[2].to_str() {
                    Some("loop") => true,
                    Some("once") => false,
                    _ => return Err("clip mode must be loop or once".into()),
                };
                clips.push((text(&v[0])?, number(&v[1])?, mode));
            }
            "--pivot" => pivot = Some([number(&v[0])?, number(&v[1])?]),
            "--attack-timing" => {
                attack_timing = Some([number(&v[0])?, number(&v[1])?, number(&v[2])?]);
            }
            "--manifest" => {
                if manifests.len() >= 11 {
                    return Err("too many manifests".into());
                }
                manifests.push(PathBuf::from(&v[0]));
            }
            _ => unreachable!(),
        }
    }
    if compose && manifests.is_empty() {
        return Err("at least one manifest required".into());
    }
    if !compose && (role.is_none() || image.is_none() || frames.is_empty()) {
        return Err("role/image/frame required".into());
    }
    Ok(Options {
        assets: assets.ok_or("assets-dir required")?,
        output: output.ok_or("output required")?,
        role: role.unwrap_or_default(),
        image: image.unwrap_or_default(),
        frames,
        clips,
        pivot,
        attack_timing,
        manifests,
    })
}
fn run(options: Options, compose: bool) -> Result<(), String> {
    publish::preflight(&options.output)?;
    let mut inputs = Vec::new();
    let (bytes, sources) = if compose {
        for path in &options.manifests {
            inputs.push(prepared::snapshot(path, 65536)?);
        }
        let (data, sources) = manifest::compose(&inputs)?;
        (data, Some(sources))
    } else {
        (
            manifest::create(
                &options.role,
                &options.image,
                &options.frames,
                &options.clips,
                options.pivot,
                options.attack_timing,
            )?,
            None,
        )
    };
    let prepared = prepared::prepare(bytes, &options.assets)?;
    let verified = sampler::verify(prepared)?;
    verified.prepared().recheck()?;
    for (path, bytes) in options.manifests.iter().zip(&inputs) {
        if prepared::snapshot(path, 65536)? != *bytes {
            return Err("source manifest changed during sampler verification".into());
        }
    }
    if let Some(warning) = publish::publish(&options.output, &verified)? {
        eprintln!("warning: {warning}");
    }
    #[derive(serde::Serialize)]
    struct Report<'a> {
        version: u32,
        #[serde(rename = "manifestBytes")]
        manifest_bytes: usize,
        #[serde(rename = "manifestSha256")]
        manifest_sha256: String,
        images: &'a [prepared::ImageReceipt],
        #[serde(skip_serializing_if = "Option::is_none")]
        roles: Option<&'a [String]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sources: Option<Vec<manifest::SourceReceipt>>,
    }
    let p = verified.prepared();
    let report = Report {
        version: 1,
        manifest_bytes: p.bytes().len(),
        manifest_sha256: prepared::digest(p.bytes()),
        images: p.images(),
        roles: if compose { Some(p.roles()) } else { None },
        sources,
    };
    println!(
        "{}",
        serde_json::to_string(&report).map_err(|e| e.to_string())?
    );
    Ok(())
}
pub fn run_cli(args: impl IntoIterator<Item = OsString>, compose: bool) -> u8 {
    let mut values = Vec::new();
    let mut total = 0usize;
    for arg in args {
        total = total.saturating_add(arg.as_encoded_bytes().len());
        if total > 65536 || values.len() >= 8192 {
            eprintln!("animation argv exceeds bounds");
            return 2;
        }
        values.push(arg);
    }
    if values.len() == 1 && values[0] == "--help" {
        println!("{USAGE}");
        return 0;
    }
    let options = match options(&values, compose) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}; {USAGE}");
            return 2;
        }
    };
    match run(options, compose) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("animation authoring refused: {e}");
            1
        }
    }
}

/// Create Scene YAML only after execution against the supplied trusted WASM.
pub fn run_scene_cli(args: impl IntoIterator<Item = OsString>) -> u8 {
    scene::run_cli(args)
}

/// Build one explicit-new offline HTML delivery with applicable real engine gates.
pub fn run_offline_cli(args: impl IntoIterator<Item = OsString>) -> u8 {
    offline::run_cli(args)
}

/// Build a portable closed recipe through the same verified offline hub.
pub fn run_recipe_cli(args: impl IntoIterator<Item = OsString>) -> u8 {
    recipe::run_cli(args)
}
/// Refresh explicit generated build assets; not an atomic release publisher.
pub fn run_copy_cli(args: impl IntoIterator<Item = OsString>) -> u8 {
    copy_assets::run_cli(args)
}

#[cfg(test)]
#[path = "coop_web_authoring/cli_tests.rs"]
mod cli_tests;

/// Create Scene YAML through the compiled native admission, without Node or WASM.
#[cfg(feature = "coop-native-authoring")]
pub fn run_native_scene_cli(args: impl IntoIterator<Item = OsString>) -> u8 {
    native_scene::run_cli(args)
}
