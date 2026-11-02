//! Optional in-process sprite package authoring. Does not install or render.
use serde::Serialize;
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};
pub mod authoring;

pub const USAGE: &str = "orr_pkg create-sprite --png FILE --license FILE --name NAME --version VERSION --out NEW_DIRECTORY (--json FILE | --atlas-size WIDTH HEIGHT [--region ID X Y WIDTH HEIGHT]... [--clip ID loop|once REGION:DURATION_MS,...]...)\nRequires Linux sprite-authoring build. Existing strict PNG/SpriteDocument admission; no --packer, fps or pivot option, shell, install, rendering or rights verification. No overwrite. Publication is visible-partial, not crash-atomic; on write failure inspect the partial output manually, do not retry/remove blindly.";
const MAX_ARGS: usize = authoring::MAX_JSON_BYTES;
pub struct Options {
    pub request: authoring::Request,
    generated: Option<Vec<u8>>,
}
#[derive(Serialize)]
struct Document {
    format: &'static str,
    version: u32,
    atlas: Atlas,
    regions: Vec<Region>,
    clips: Vec<Clip>,
}
#[derive(Serialize)]
struct Atlas {
    image: &'static str,
    width: i128,
    height: i128,
}
#[derive(Serialize)]
struct Region {
    id: i128,
    x: i128,
    y: i128,
    width: i128,
    height: i128,
}
#[derive(Serialize)]
struct Clip {
    id: String,
    mode: String,
    frames: Vec<Frame>,
}
#[derive(Serialize)]
struct Frame {
    region: i128,
    duration_ms: i128,
}
fn integer(text: &str) -> Result<i128, String> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if text.len() > 21 || digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err("expected a bounded decimal integer".into());
    }
    text.parse()
        .map_err(|_| "expected a bounded decimal integer".into())
}
fn text(value: &OsString) -> Result<&str, String> {
    value
        .to_str()
        .ok_or_else(|| "authoring text must be UTF-8".into())
}
impl Options {
    pub fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut args = Vec::new();
        let mut total = 0usize;
        for arg in arguments {
            total = total
                .checked_add(arg.as_encoded_bytes().len() + 1)
                .ok_or("argument byte overflow")?;
            if total > MAX_ARGS
                || args.len() >= orr_sprite::MAX_REGIONS * 6 + orr_sprite::MAX_CLIPS * 4 + 32
            {
                return Err("arguments exceed bounded authoring budget".into());
            }
            args.push(arg);
        }
        let mut unique = BTreeMap::<String, Vec<OsString>>::new();
        let mut regions = Vec::new();
        let mut clips = Vec::new();
        let mut total_frames = 0;
        let mut i = 0;
        while i < args.len() {
            let flag = text(&args[i])?;
            i += 1;
            let n=match flag { "--region"=>5,"--clip"=>3,"--atlas-size"=>2,
                "--png"|"--json"|"--license"|"--name"|"--version"|"--out"=>1,
                _=>return Err(format!("unknown or obsolete option {flag}; no external executable, fps or pivot options")), };
            let values = args.get(i..i + n).ok_or("missing option values")?;
            i += n;
            match flag {
                "--region" => {
                    if regions.len() >= orr_sprite::MAX_REGIONS {
                        return Err("too many regions".into());
                    }
                    let ns: Vec<_> = values
                        .iter()
                        .map(|v| integer(text(v)?))
                        .collect::<Result<_, String>>()?;
                    regions.push(Region {
                        id: ns[0],
                        x: ns[1],
                        y: ns[2],
                        width: ns[3],
                        height: ns[4],
                    });
                }
                "--clip" => {
                    if clips.len() >= orr_sprite::MAX_CLIPS {
                        return Err("too many clips".into());
                    }
                    let sequence = text(&values[2])?;
                    let count = sequence.bytes().filter(|b| *b == b',').count() + 1;
                    total_frames += count;
                    if count > orr_sprite::MAX_FRAMES_PER_CLIP
                        || total_frames > orr_sprite::MAX_TOTAL_FRAMES
                    {
                        return Err("too many frame references".into());
                    }
                    let frames = sequence
                        .split(',')
                        .map(|pair| {
                            let (region, duration) = pair
                                .split_once(':')
                                .ok_or("expected REGION:DURATION_MS pairs")?;
                            Ok(Frame {
                                region: integer(region)?,
                                duration_ms: integer(duration)?,
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    clips.push(Clip {
                        id: text(&values[0])?.into(),
                        mode: text(&values[1])?.into(),
                        frames,
                    });
                }
                _ => {
                    if unique.insert(flag.into(), values.to_vec()).is_some() {
                        return Err(format!("duplicate option {flag}"));
                    }
                }
            }
        }
        let direct = unique.remove("--json");
        let size = unique.remove("--atlas-size");
        let (json,generated)=match (direct,size) {
            (Some(path),None) if regions.is_empty() && clips.is_empty()=>(PathBuf::from(&path[0]),None),
            (None,Some(size))=>{
                let doc=Document{format:"orr_sprite",version:1,atlas:Atlas{image:"atlas.png",width:integer(text(&size[0])?)?,height:integer(text(&size[1])?)?},regions,clips};
                let mut bytes=serde_json::to_vec_pretty(&doc).map_err(|e|e.to_string())?;bytes.push(b'\n');
                if bytes.len()>authoring::MAX_JSON_BYTES{return Err("sprite JSON exceeds 4 MiB".into());}
                (PathBuf::new(),Some(bytes))
            }
            _=>return Err("choose exactly --json or --atlas-size with explicit regions/clips; modes cannot be mixed".into()),
        };
        let mut take = |name: &str| {
            unique
                .remove(name)
                .map(|v| v[0].clone())
                .ok_or_else(|| format!("missing {name}"))
        };
        let request = authoring::Request {
            png: take("--png")?.into(),
            json,
            license: take("--license")?.into(),
            name: take("--name")?
                .into_string()
                .map_err(|_| "name must be UTF-8")?,
            version: take("--version")?
                .into_string()
                .map_err(|_| "version must be UTF-8")?,
            out: take("--out")?.into(),
        };
        Ok(Self { request, generated })
    }
}
pub fn create(options: &Options) -> Result<authoring::Report, String> {
    match &options.generated {
        Some(json) => authoring::import_package_bytes(&options.request, json),
        None => authoring::import_package(&options.request),
    }
}
pub fn run_cli(arguments: impl IntoIterator<Item = OsString>) -> u8 {
    let mut args = arguments.into_iter().peekable();
    if args.peek().is_some_and(|a| a == "--help") {
        args.next();
        if args.next().is_none() {
            println!("{USAGE}");
            return 0;
        }
        return 2;
    }
    let options = match Options::parse(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("orr_pkg create-sprite: {e}\n{USAGE}");
            return 2;
        }
    };
    match create(&options) {
        Ok(report) => {
            println!("Created {}: {} ({}x{}, {} decoded RGBA bytes). Package bytes only; rendering/playback/rights not verified.",options.request.out.display(),report.image,report.width,report.height,report.rgba_bytes);
            0
        }
        Err(e) => {
            eprintln!("Sprite package creation failed: {e}");
            1
        }
    }
}
#[cfg(test)]
mod tests;
