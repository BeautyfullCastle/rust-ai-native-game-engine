//! Read-only saved-scene CLI. Capture is presentation only, never navigation input.
use crate::authored::AuthoredPlayer;
use std::{
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
};

pub const USAGE: &str = "navigation_player --scene FILE --ticks 0..100000 [--restart-check] [--capture NEW.png]\nReads an authored point-agent NavigationYard3D scene; no input, network, editor or bundle publishing. Capture requires gpu. Terrain-triangle point agent only: no radius clearance, avoidance or physical collision guarantee.";

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub scene: PathBuf,
    pub ticks: u64,
    pub restart_check: bool,
    pub capture: Option<PathBuf>,
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Option<Options>, String> {
    let mut bounded = Vec::new();
    let mut bytes = 0usize;
    for arg in args {
        bytes = bytes
            .checked_add(arg.len())
            .ok_or("argument size overflow")?;
        if bounded.len() == 8 || bytes > 16 * 1024 {
            return Err("argument count/size limit exceeded".into());
        }
        bounded.push(arg);
    }
    let args = bounded;
    if args.len() == 1 && args[0] == "--help" {
        return Ok(None);
    }
    let (mut scene, mut ticks, mut capture, mut restart_check) = (None, None, None, false);
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        match flag.to_str() {
            Some("--scene") => {
                if scene.is_some() {
                    return Err("duplicate --scene".into());
                }
                let value = args.next().ok_or("missing scene path")?;
                if value.is_empty() || value.len() > 4096 {
                    return Err("scene path must be 1..4096 bytes".into());
                }
                scene = Some(PathBuf::from(value));
            }
            Some("--ticks") => {
                if ticks.is_some() {
                    return Err("duplicate --ticks".into());
                }
                let value = args.next().ok_or("missing ticks")?;
                let text = value.to_str().ok_or("ticks must be decimal")?;
                if text.is_empty() || !text.bytes().all(|c| c.is_ascii_digit()) {
                    return Err("ticks must be decimal 0..100000".into());
                }
                let count = text.parse::<u64>().map_err(|_| "ticks overflow")?;
                if count > 100_000 {
                    return Err("ticks exceeds 100000".into());
                }
                ticks = Some(count);
            }
            Some("--restart-check") => {
                if restart_check {
                    return Err("duplicate --restart-check".into());
                }
                restart_check = true;
            }
            Some("--capture") => {
                if !cfg!(feature = "gpu") {
                    return Err("--capture requires the gpu feature".into());
                }
                if capture.is_some() {
                    return Err("duplicate --capture".into());
                }
                let value = args.next().ok_or("missing capture path")?;
                if value.is_empty() || value.len() > 4096 {
                    return Err("capture path must be 1..4096 bytes".into());
                }
                capture = Some(PathBuf::from(value));
            }
            _ => return Err("unknown option".into()),
        }
    }
    Ok(Some(Options {
        scene: scene.ok_or("--scene is required")?,
        ticks: ticks.ok_or("--ticks is required")?,
        restart_check,
        capture,
    }))
}

fn capture_path(path: &Path) -> Result<PathBuf, String> {
    if path.extension().and_then(|v| v.to_str()) != Some("png") {
        return Err("capture output must end in .png".into());
    }
    if path
        .components()
        .any(|v| matches!(v, Component::ParentDir | Component::CurDir))
    {
        return Err("capture path must not contain traversal components".into());
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let parent = absolute
        .parent()
        .ok_or("capture needs an existing parent directory")?;
    let mut current = PathBuf::new();
    for part in parent.components() {
        current.push(part.as_os_str());
        let metadata =
            fs::symlink_metadata(&current).map_err(|e| format!("capture parent: {e}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("capture parent must be a non-symlink directory".into());
        }
    }
    match fs::symlink_metadata(&absolute) {
        Ok(_) => return Err("capture output already exists; never overwritten".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("capture preflight: {e}")),
    }
    Ok(absolute)
}

fn hex(bytes: [u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn run(options: Options) -> Result<serde_json::Value, String> {
    // Public callers must receive exactly the same limits as command-line callers.
    if options.ticks > 100_000 {
        return Err("ticks exceeds 100000".into());
    }
    if options.scene.as_os_str().is_empty() || options.scene.as_os_str().len() > 4096 {
        return Err("scene path must be 1..4096 bytes".into());
    }
    if let Some(path) = &options.capture {
        if !cfg!(feature = "gpu") {
            return Err("capture requires gpu".into());
        }
        if path.as_os_str().is_empty() || path.as_os_str().len() > 4096 {
            return Err("capture path must be 1..4096 bytes".into());
        }
    }
    // Refuse bad output before opening the scene or stepping authoritative state.
    let capture = options.capture.as_deref().map(capture_path).transpose()?;
    let mut player = AuthoredPlayer::open(&options.scene)?;
    // At most 100001 u64 checksums (800008 bytes), only when requested.
    let mut trajectory = if options.restart_check {
        Vec::with_capacity(options.ticks as usize + 1)
    } else {
        Vec::new()
    };
    if options.restart_check {
        trajectory.push(player.checksum());
    }
    for _ in 0..options.ticks {
        player.step()?;
        if options.restart_check {
            trajectory.push(player.checksum());
        }
    }
    if options.restart_check {
        let expected = player.frame().to_bytes();
        player.restart()?;
        if player.checksum() != trajectory[0] {
            return Err("restart initial checksum diverged".into());
        }
        for checksum in trajectory.iter().skip(1) {
            player.step()?;
            if player.checksum() != *checksum {
                return Err(format!(
                    "restart trajectory diverged at tick {}",
                    player.tick()
                ));
            }
        }
        if player.frame().to_bytes() != expected {
            return Err("restart final Frame bytes diverged".into());
        }
    }
    let view = player.view()?;
    let capture_receipt = match capture {
        None => serde_json::Value::Null,
        Some(path) => {
            #[cfg(feature = "gpu")]
            {
                capture_png(&path, &view)?
            }
            #[cfg(not(feature = "gpu"))]
            {
                let _ = path;
                return Err("capture requires gpu".into());
            }
        }
    };
    let result = serde_json::json!({
        "schema":1,"tick":player.tick(),"checksum":player.checksum(),
        "status":format!("{:?}",view.navigator.status()),"position_raw":view.navigator.position().map(|v|v.raw()),
        "terrain_asset_id":view.terrain.asset_id(),"terrain_revision":hex(view.terrain.revision()),
        "graph_revision":hex(view.graph.revision()),"graph_terrain_revision":hex(view.graph.terrain_revision()),
        "restart_checked":options.restart_check,"point_agent_only":true,
        "limitations":"terrain-triangle point agent; no radius clearance, dynamic avoidance, physics collision or window input",
        "capture":capture_receipt
    });
    Ok(result)
}

#[cfg(feature = "gpu")]
fn capture_png(
    path: &Path,
    view: &orr_remote::navigation_yard3d::NavigationView,
) -> Result<serde_json::Value, String> {
    use orr_render::orr_rhi::{Rhi, Wgpu, WgpuOptions};
    use std::io::Write;
    let gpu = Wgpu::headless(WgpuOptions::default()).map_err(|e| e.to_string())?;
    let mut renderer = crate::gpu::renderer(
        &gpu,
        &view.terrain,
        view.navigator.path(),
        view.navigator.position(),
        crate::PathDisplay::Current,
    )
    .map_err(|e| e.to_string())?;
    let size = (1000, 750);
    let pixels = crate::gpu::capture(
        &mut renderer,
        &crate::gpu::top_camera(),
        &crate::gpu::lighting(),
        size,
    )
    .map_err(|e| e.to_string())?;
    if pixels.len() != 1000 * 750 * 4 {
        return Err("capture RGBA size mismatch".into());
    }
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, size.0, size.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&pixels)
            .map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
    }
    if encoded.len() > 16 * 1024 * 1024 {
        return Err("capture PNG exceeds bounded output".into());
    }
    capture_path(path)?;
    // Trusted, quiescent ancestry contract: no hostile concurrent rename sandbox.
    // create_new refuses dangling links/existing outputs. Never recursive cleanup.
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("capture not published: {e}"))?;
    output
        .write_all(&encoded)
        .map_err(|e| format!("capture output may be partial; preserved for inspection: {e}"))?;
    output
        .sync_all()
        .map_err(|e| format!("capture published but durability uncertain; do not retry: {e}"))?;
    fs::File::open(path.parent().ok_or("capture parent missing")?)
        .and_then(|f| f.sync_all())
        .map_err(|e| {
            format!("capture published but parent durability uncertain; do not retry: {e}")
        })?;
    Ok(
        serde_json::json!({"path":path,"width":size.0,"height":size.1,"rgba_bytes":pixels.len(),"png_bytes":encoded.len(),"adapter":gpu.adapter_name(),"software":gpu.is_software(),"offscreen":true}),
    )
}
