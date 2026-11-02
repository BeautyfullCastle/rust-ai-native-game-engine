//! Closed offline build consumer: no build/fetch/Python, no generic JS bundling.
mod audio;
mod bundle;
mod html_template;
use super::{absolute, manifest, prepared, publish, sampler, scene};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
};
const MAX_INPUT: usize = 128 * 1024 * 1024;
pub(super) const MAX_OUTPUT: usize = 256 * 1024 * 1024;
const MAX_ASSET: usize = 64 * 1024 * 1024;
const USAGE:&str="orr_pkg pack-coop-offline --build-dir TRUSTED_BUILD --output NEW_HTML [--scene FILE] [--animations FILE] [--audio BANK_JSON]\nPython/Pillow-free; trusted Node is required for authored Scene/animation. Executes supplied trusted WASM/build sampler, not a sandbox. Always explicit new output; no legacy overwrite.";
#[derive(Debug)]
struct Options {
    root: PathBuf,
    output: PathBuf,
    scene: Option<PathBuf>,
    animations: Option<PathBuf>,
    audio: Option<PathBuf>,
}
fn options(args: &[OsString]) -> Result<Options, String> {
    if args.len() > 10
        || args.len() % 2 != 0
        || args
            .iter()
            .any(|s| s.is_empty() || s.as_encoded_bytes().len() > 4096)
    {
        return Err("bounded option/value pairs required".into());
    }
    let (mut root, mut output, mut scene, mut animations, mut audio) =
        (None, None, None, None, None);
    let mut seen = BTreeSet::new();
    for pair in args.chunks_exact(2) {
        let key = pair[0].to_str().ok_or("non-UTF8 flag")?;
        if !seen.insert(key) {
            return Err("duplicate option".into());
        }
        let path = PathBuf::from(&pair[1]);
        match key {
            "--build-dir" => root = Some(path),
            "--output" => output = Some(path),
            "--scene" => scene = Some(path),
            "--animations" => animations = Some(path),
            "--audio" => audio = Some(path),
            _ => return Err("unknown offline option".into()),
        }
    }
    Ok(Options {
        root: root.ok_or("--build-dir required")?,
        output: output.ok_or("--output NEW required")?,
        scene,
        animations,
        audio,
    })
}
fn take(path: &Path, max: usize, total: &mut usize) -> Result<Vec<u8>, String> {
    let remaining = MAX_INPUT
        .checked_sub(*total)
        .ok_or("input aggregate exceeded")?;
    let data = prepared::snapshot(path, max.min(remaining))?;
    *total = total.checked_add(data.len()).ok_or("input overflow")?;
    Ok(data)
}
/// Construction is private: only the fixed closed asset inventory can enter a build gate.
pub(super) struct BuildSnapshot {
    root: PathBuf,
    files: BTreeMap<String, Vec<u8>>,
}
impl BuildSnapshot {
    fn open(root: &Path, total: &mut usize) -> Result<Self, String> {
        let root = absolute(root)?;
        let mut files = BTreeMap::new();
        for name in bundle::ASSETS {
            files.insert(name.to_owned(), take(&root.join(name), MAX_ASSET, total)?);
        }
        Ok(Self { root, files })
    }
    pub(super) fn file(&self, name: &str) -> &[u8] {
        &self.files[name]
    }
    fn recheck(&self) -> Result<(), String> {
        for (name, bytes) in &self.files {
            if prepared::snapshot(&self.root.join(name), MAX_ASSET)? != *bytes {
                return Err("build changed after snapshot".into());
            }
        }
        Ok(())
    }
}
#[derive(Default)]
struct ExtraInputs(BTreeMap<PathBuf, Vec<u8>>);
impl ExtraInputs {
    fn get(&mut self, path: &Path, max: usize, total: &mut usize) -> Result<Vec<u8>, String> {
        let path = absolute(path)?;
        if let Some(bytes) = self.0.get(&path) {
            if bytes.len() > max {
                return Err("input exceeds role bound".into());
            }
            return Ok(bytes.clone());
        }
        let bytes = take(&path, max, total)?;
        self.0.insert(path, bytes.clone());
        Ok(bytes)
    }
    fn recheck(&self) -> Result<(), String> {
        for (path, bytes) in &self.0 {
            if prepared::snapshot(path, bytes.len())? != *bytes {
                return Err("optional input changed after validation".into());
            }
        }
        Ok(())
    }
}
/// No public constructor: only complete applicable real gate proofs produce this value.
pub(super) struct OfflineHtml {
    bytes: Vec<u8>,
}
impl OfflineHtml {
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
fn append(out: &mut String, value: &str) -> Result<(), String> {
    if out
        .len()
        .checked_add(value.len())
        .ok_or("HTML byte overflow")?
        > MAX_OUTPUT
    {
        return Err("HTML exceeds256MiB".into());
    }
    out.push_str(value);
    Ok(())
}
fn binary(out: &mut String, id: &str, bytes: &[u8]) -> Result<(), String> {
    let encoded = bytes
        .len()
        .checked_add(2)
        .and_then(|n| n.checked_div(3))
        .and_then(|n| n.checked_mul(4))
        .ok_or("base64 size overflow")?;
    let extra = encoded
        .checked_add(id.len())
        .and_then(|n| n.checked_add(64))
        .ok_or("HTML size overflow")?;
    if out.len().checked_add(extra).ok_or("HTML size overflow")? > MAX_OUTPUT {
        return Err("HTML exceeds256MiB".into());
    }
    append(
        out,
        &format!("<script type=\"application/octet-stream\" id=\"{id}\">"),
    )?;
    STANDARD.encode_string(bytes, out);
    append(out, "</script>")
}
fn json_script(out: &mut String, id: &str, json: &str) -> Result<(), String> {
    let replacements = json.bytes().filter(|b| *b == b'<').count();
    let size = json
        .len()
        .checked_add(replacements.checked_mul(5).ok_or("JSON size overflow")?)
        .ok_or("JSON size overflow")?;
    if size > MAX_OUTPUT
        || out
            .len()
            .checked_add(size)
            .and_then(|n| n.checked_add(id.len() + 64))
            .ok_or("HTML size overflow")?
            > MAX_OUTPUT
    {
        return Err("HTML exceeds256MiB".into());
    }
    append(
        out,
        &format!("<script type=\"application/json\" id=\"{id}\">"),
    )?;
    append(out, &json.replace('<', "\\u003c"))?;
    append(out, "</script>")
}
fn generate(o: &Options) -> Result<OfflineHtml, String> {
    let mut total = 0;
    let build = BuildSnapshot::open(&o.root, &mut total)?;
    let mut extra = ExtraInputs::default();
    let js = bundle::bundle(&build.files)?;
    let template = html_template::inspect(build.file("index.html"))?;
    let css = std::str::from_utf8(build.file("style.css")).map_err(|e| e.to_string())?;
    if regex::Regex::new("(?i)</style")
        .map_err(|e| e.to_string())?
        .is_match(css)
    {
        return Err("CSS terminates style element".into());
    }
    let scene = match &o.scene {
        Some(path) => Some(scene::verify_bytes(
            extra.get(path, 65536, &mut total)?,
            build.file("game.wasm"),
        )?),
        None => None,
    };
    let animation = if let Some(path) = &o.animations {
        let bytes = extra.get(path, 65536, &mut total)?;
        let value = manifest::parse(&bytes)?;
        let names = manifest::names(&value)?;
        let mut images = BTreeMap::new();
        for name in names {
            let fixed = format!("assets/{name}");
            let data = if let Some(data) = build.files.get(&fixed) {
                if data.len() > 8 * 1024 * 1024 {
                    return Err("animation image exceeds8MiB".into());
                }
                data.clone()
            } else {
                extra.get(
                    &build.root.join("assets").join(&name),
                    8 * 1024 * 1024,
                    &mut total,
                )?
            };
            images.insert(name, data);
        }
        let p = prepared::prepare_images(bytes, &build.root.join("assets"), images)?;
        Some(sampler::verify_build(p, &build)?)
    } else {
        None
    };
    let audio = if let Some(path) = &o.audio {
        let bytes = extra.get(path, 128 * 1024, &mut total)?;
        let root = absolute(path)?
            .parent()
            .ok_or("audio parent missing")?
            .to_owned();
        let mut objects = BTreeMap::new();
        for cue in audio::CUES {
            let name = format!("{cue}.pcm");
            objects.insert(
                name.clone(),
                extra.get(&root.join(name), 96008, &mut total)?,
            );
        }
        Some(audio::prepare(&bytes, &objects)?)
    } else {
        None
    };
    // Preserve original JSON bytes so duplicate-last-wins, large integers and unknown fields
    // are not rewritten through f64 or a sorted map. Nonfinite/deep JSON is explicitly refused.
    let metadata =
        std::str::from_utf8(build.file("assets/characters.json")).map_err(|e| e.to_string())?;
    let _: serde_json::Value = serde_json::from_str(metadata)
        .map_err(|e| format!("invalid finite character metadata: {e}"))?;
    let mut embedded = String::new();
    binary(&mut embedded, "wasm-core", build.file("game.wasm"))?;
    if let Some(audio) = audio {
        append(&mut embedded, audio.embedded())?;
    }
    json_script(&mut embedded, "character-atlas-meta", metadata)?;
    binary(&mut embedded, "tower-atlas", build.file("assets/units.png"))?;
    binary(
        &mut embedded,
        "enemy-atlas",
        build.file("assets/enemies.png"),
    )?;
    if let Some(scene) = scene {
        binary(&mut embedded, "authored-scene", scene.bytes())?;
        json_script(
            &mut embedded,
            "authored-scene-receipt",
            &serde_json::to_string(scene.receipt()).map_err(|e| e.to_string())?,
        )?;
    }
    if let Some(animation) = animation {
        if !animation.matches(&build) {
            return Err("build sampler proof mismatch".into());
        }
        let p = animation.prepared();
        binary(&mut embedded, "authored-animation-manifest", p.bytes())?;
        #[derive(Serialize)]
        struct Receipt<'a> {
            version: u32,
            #[serde(rename = "manifestBytes")]
            bytes: usize,
            #[serde(rename = "manifestSha256")]
            sha: String,
            images: &'a [prepared::ImageReceipt],
        }
        let receipt = Receipt {
            version: 1,
            bytes: p.bytes().len(),
            sha: prepared::digest(p.bytes()),
            images: p.images(),
        };
        json_script(
            &mut embedded,
            "authored-animation-receipt",
            &serde_json::to_string(&receipt).map_err(|e| e.to_string())?,
        )?;
        for image in p.images() {
            binary(&mut embedded, &image.id, p.image_bytes(&image.name))?;
        }
    }
    let html = template.render(css, &embedded, &js)?;
    if html.len() > MAX_OUTPUT {
        return Err("HTML exceeds256MiB".into());
    }
    build.recheck()?;
    extra.recheck()?;
    Ok(OfflineHtml {
        bytes: html.into_bytes(),
    })
}
pub(super) fn run_cli(args: impl IntoIterator<Item = OsString>) -> u8 {
    let args: Vec<_> = args.into_iter().take(12).collect();
    if args.len() == 1 && args[0] == "--help" {
        println!("{USAGE}");
        return 0;
    }
    let o = match options(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}; {USAGE}");
            return 2;
        }
    };
    match (|| {
        publish::preflight(&o.output)?;
        let html = generate(&o)?;
        publish::publish_offline(&o.output, &html)
    })() {
        Ok(warning) => {
            if let Some(w) = warning {
                eprintln!("warning: {w}");
            }
            println!("{}", o.output.display());
            0
        }
        Err(e) => {
            eprintln!("offline packing refused: {e}");
            1
        }
    }
}
#[cfg(test)]
#[path = "offline/tests.rs"]
mod tests;
