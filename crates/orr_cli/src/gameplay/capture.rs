//! Bounded, single-shot framebuffer capture to a no-clobber local PNG.
use super::*;
use orr_remote::link::{QueueLimits, TransportQueueLimits};
use std::{
    io::{Cursor, Write},
    path::{Path, PathBuf},
};
const PNG_LIMIT: usize = 4 * 1024 * 1024;
const BASE64_LIMIT: usize = PNG_LIMIT.div_ceil(3) * 4;
const PIXELS: u64 = 1_048_576;
struct Options {
    path: PathBuf,
    timeout: Duration,
    width: u32,
    height: u32,
}
fn invalid() -> CliErr {
    CliErr::Usage("invalid capture options or destination; see `orr help gameplay`".into())
}
fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}
fn validate_path(path: &Path) -> Result<(), CliErr> {
    if path.as_os_str().is_empty() || path.file_name().is_none() {
        return Err(invalid());
    }
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err(invalid()),
    }
    // Reject symlink ancestors, including broken links, before opening a socket.
    let mut cursor = PathBuf::new();
    for part in parent(path).components() {
        cursor.push(part);
        let metadata = std::fs::symlink_metadata(&cursor).map_err(|_| invalid())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(invalid());
        }
    }
    Ok(())
}
fn options(args: &[String]) -> Result<Options, CliErr> {
    let flags = [
        ("--out", true),
        ("--timeout-ms", true),
        ("--max-width", true),
        ("--max-height", true),
    ];
    let p = parse(args, &flags, false).map_err(|_| invalid())?;
    if p.pos != ["capture"] || flags.iter().any(|(f, _)| p.all(f).len() > 1) {
        return Err(invalid());
    }
    let path = PathBuf::from(p.get("--out").ok_or_else(invalid)?);
    validate_path(&path)?;
    let number = |flag, default: &str, lo, hi| -> Result<u64, CliErr> {
        p.get(flag)
            .unwrap_or(default)
            .parse::<u64>()
            .ok()
            .filter(|n| (lo..=hi).contains(n))
            .ok_or_else(invalid)
    };
    Ok(Options {
        path,
        timeout: Duration::from_millis(number("--timeout-ms", "10000", 100, 60000)?),
        width: number("--max-width", "2048", 1, 2048)? as u32,
        height: number("--max-height", "2048", 1, 2048)? as u32,
    })
}
fn validate(v: &J, identity: &J, o: &Options) -> Result<Vec<u8>, CliErr> {
    let bad = || failed("invalid screenshot response; no PNG saved");
    let width = v["width"].as_u64().ok_or_else(bad)?;
    let height = v["height"].as_u64().ok_or_else(bad)?;
    if v["status"] != "captured"
        || v["source"] != "editor.app_framebuffer"
        || v["mime_type"] != "image/png"
        || v["game"] != identity["game"]
        || v["build_id"] != identity["build_id"]
        || !matches!(v["mode"].as_str(), Some("edit" | "play"))
        || !v["paused"].is_boolean()
        || !checksum(&v["checksum"])
        || ["tick", "epoch", "frame_seq", "ui_frame"]
            .iter()
            .any(|k| decimal(&v[k]).is_none())
        || width == 0
        || height == 0
        || width > u64::from(o.width)
        || height > u64::from(o.height)
        || width * height > PIXELS
    {
        return Err(bad());
    }
    let encoded = v["png_base64"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= BASE64_LIMIT)
        .ok_or_else(bad)?;
    let bytes = orr_remote::codec::b64_decode(encoded)
        .filter(|b| b.len() <= PNG_LIMIT)
        .ok_or_else(bad)?;
    if !bytes.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 174, 66, 96, 130]) {
        return Err(bad());
    }
    let mut decode_options = png::DecodeOptions::default();
    decode_options.set_ignore_checksums(false);
    decode_options.set_skip_ancillary_crc_failures(false);
    let mut input = Cursor::new(&bytes);
    let mut decoder = png::Decoder::new_with_options(&mut input, decode_options);
    decoder.set_limits(png::Limits {
        bytes: 16 * 1024 * 1024,
    });
    let mut reader = decoder.read_info().map_err(|_| bad())?;
    if u64::from(reader.info().width) != width
        || u64::from(reader.info().height) != height
        || reader.info().animation_control.is_some()
    {
        return Err(bad());
    }
    let size = reader
        .output_buffer_size()
        .filter(|n| *n <= 8 * PIXELS as usize)
        .ok_or_else(bad)?;
    let mut pixels = vec![0; size];
    reader.next_frame(&mut pixels).map_err(|_| bad())?;
    reader.finish().map_err(|_| bad())?;
    drop(reader);
    if input.position() != bytes.len() as u64 {
        return Err(bad());
    }
    Ok(bytes)
}
fn save(path: &Path, bytes: &[u8]) -> Result<(), CliErr> {
    let write = || -> std::io::Result<()> {
        let mut staged = tempfile::Builder::new()
            .prefix(".orr-capture-")
            .tempfile_in(parent(path))?;
        staged.write_all(bytes)?;
        staged.as_file().sync_all()?;
        staged.persist_noclobber(path).map_err(|e| e.error)?;
        Ok(())
    };
    write().map_err(|_| failed("PNG could not be saved without replacement"))
}
fn execute(s: &mut impl Session, o: &Options, d: Instant) -> Result<J, CliErr> {
    let identity = request(s, d, "editor.gameplay.describe", json!({}))?;
    if identity["game"]
        .as_str()
        .is_none_or(|s| s.is_empty() || s.len() > 256)
        || !checksum(&identity["build_id"])
    {
        return Err(failed("invalid game identity"));
    }
    let v = request(
        s,
        d,
        "view.screenshot",
        json!({"target":"app_framebuffer","max_width":o.width,"max_height":o.height}),
    )?;
    let bytes = validate(&v, &identity, o)?;
    remaining(d)?;
    save(&o.path, &bytes)?;
    let mut result = json!({"local_png_path":o.path,"bytes":bytes.len()});
    for key in [
        "status",
        "source",
        "game",
        "build_id",
        "mode",
        "paused",
        "tick",
        "epoch",
        "checksum",
        "frame_seq",
        "ui_frame",
        "width",
        "height",
        "mime_type",
    ] {
        result[key] = v[key].clone();
    }
    Ok(result)
}
fn output_json(ctx: &Ctx, mut result: J) -> String {
    // Redact decoded field values, not JSON text: secrets may contain escapes.
    for value in result
        .as_object_mut()
        .expect("capture metadata object")
        .values_mut()
    {
        if let Some(text) = value.as_str() {
            *value = J::String(ctx.redact(text));
        }
    }
    serde_json::to_string_pretty(&result).unwrap()
}
pub(super) fn run(ctx: &mut Ctx, args: &[String]) -> Result<(), CliErr> {
    let o = options(args)?;
    let d = Instant::now() + o.timeout;
    let limits = TransportQueueLimits {
        incoming: QueueLimits {
            max_items: 64,
            max_bytes: 6 * 1024 * 1024,
        },
        outgoing: QueueLimits {
            max_items: 16,
            max_bytes: 64 * 1024,
        },
    };
    let transport = PumpedWs::connect_with_queue_limits(&ctx.url, remaining(d)?, limits)
        .map_err(|_| CliErr::Conn("capture connection failed; no reconnect attempted".into()))?;
    let mut s = ErpClient::with_transport(Box::new(transport));
    if let Some(token) = ctx.token.as_deref() {
        Session::call(&mut s, "auth", json!({"token":token}), remaining(d)?)?;
    }
    let result = execute(&mut s, &o, d)?;
    // Unlike the general CLI stdout helper, surface a partial-success error.
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout,"{}",output_json(ctx, result)).and_then(|()|stdout.flush())
        .map_err(|_|CliErr::Erp("PNG saved, but capture metadata could not be written to stdout; do not retry the same destination".into()))?;
    Ok(())
}
#[cfg(test)]
mod tests;
