//! Read-only saved-scene CLI. Capture is presentation only, never navigation input.
use crate::authored::AuthoredPlayer;
use orr_terrain_view::surface::Surface;

/// Presentation-only companion. The existing AuthoredPlayer is the sole simulation owner.
pub struct SurfacePlayer {
    authored: AuthoredPlayer,
    surface: Surface,
}
impl SurfacePlayer {
    pub fn open(scene: &Path) -> Result<Self, String> {
        let scene = crate::surface_bundle::io::checked_path(scene)?;
        let authored = AuthoredPlayer::open(&scene)?;
        let relative = authored
            .frame()
            .singleton::<orr_remote::navigation_yard3d::NavigationScenePin>()
            .source()?;
        let sidecar = orr_terrain_view::surface::sidecar_relative(relative)?;
        let snapshot = crate::surface_bundle::io::Snapshot::read(
            &scene.parent().ok_or("scene parent missing")?.join(sidecar),
            crate::surface_bundle::MAX_SURFACE,
        )?;
        let surface = Surface::from_bytes(&snapshot.bytes)?;
        let player = Self::from_verified(authored, surface)?;
        snapshot.recheck()?;
        Ok(player)
    }
    /// Bundle admission supplies the decoded verified snapshot, never reopens its path.
    pub fn from_verified(authored: AuthoredPlayer, surface: Surface) -> Result<Self, String> {
        let relative = authored
            .frame()
            .singleton::<orr_remote::navigation_yard3d::NavigationScenePin>()
            .source()?;
        surface.validate_terrain(relative, &authored.view()?.terrain)?;
        Ok(Self { authored, surface })
    }
    pub fn surface(&self) -> &Surface {
        &self.surface
    }
    pub fn frame(&self) -> &orr_ecs::Frame {
        self.authored.frame()
    }
    pub fn checksum(&self) -> u64 {
        self.authored.checksum()
    }
    pub fn tick(&self) -> u64 {
        self.authored.tick()
    }
    pub fn step(&mut self) -> Result<(), String> {
        self.authored.step()
    }
    pub fn restart(&mut self) -> Result<(), String> {
        self.authored.restart()
    }
    pub fn view(&self) -> Result<orr_remote::navigation_yard3d::NavigationView, String> {
        self.authored.view()
    }
    pub fn model(&self) -> Result<orr_model::StaticModel, String> {
        let view = self.view()?;
        let legacy = crate::to_static_model(
            &view.terrain,
            view.navigator.path(),
            view.navigator.position(),
            crate::PathDisplay::Current,
        )
        .map_err(|e| e.to_string())?;
        let terrain =
            orr_terrain_view::surface::to_static_model(&view.terrain, &[], &self.surface)?
                .ok_or("no terrain geometry")?;
        let mut source = terrain.source().clone();
        let image_offset = source.images.len() as u32;
        let material_offset = source.materials.len() as u32;
        source.images.extend(legacy.source().images.iter().cloned());
        source.materials.extend(
            legacy
                .source()
                .materials
                .iter()
                .cloned()
                .map(|mut material| {
                    material.image += image_offset;
                    material
                }),
        );
        // The legacy bridge emits terrain first, then its exact route and agent.
        source
            .primitives
            .extend(
                legacy
                    .source()
                    .primitives
                    .iter()
                    .skip(1)
                    .cloned()
                    .map(|mut primitive| {
                        primitive.material += material_offset;
                        primitive
                    }),
            );
        orr_model::StaticModel::new(source).map_err(|e| e.to_string())
    }
}

use std::{
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
};

pub const USAGE: &str = "navigation_surface_player (--scene FILE | --bundle DIR) --ticks 0..100000 [--restart-check] [--capture NEW.png]\nReads an authored point-agent NavigationYard3D scene; no input, network, editor or bundle publishing. Capture requires gpu and the explicit surface profile. --capabilities reports compiled admission profiles. Terrain-triangle point agent only: no radius clearance, avoidance or physical collision guarantee.";

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

/// Keep legacy Options/parse callers source-compatible; bundle never falls back to scene admission.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Scene(Options),
    Bundle(Options),
    Capabilities,
}
pub fn capabilities() -> &'static str {
    if cfg!(feature = "gpu") {
        "ORR_NAVIGATION_SURFACE_PLAYER_V2\nORR_NAVIGATION_SURFACE_CAPTURE_V2"
    } else {
        "ORR_NAVIGATION_SURFACE_PLAYER_V2"
    }
}
pub fn parse_command(args: impl IntoIterator<Item = OsString>) -> Result<Option<Command>, String> {
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
    if bounded.len() == 1 && bounded[0] == "--capabilities" {
        return Ok(Some(Command::Capabilities));
    }
    let mut bundle = false;
    let mut i = 0;
    while i < bounded.len() {
        if bounded[i] == "--bundle" {
            if !cfg!(all(
                feature = "authored-surface-bundle",
                target_arch = "x86_64"
            )) {
                return Err("--bundle requires authored-surface-bundle".into());
            }
            bundle = true;
            bounded[i] = OsString::from("--scene");
        }
        i += if bounded[i] == "--restart-check" {
            1
        } else {
            2
        };
    }
    let options = parse(bounded)?;
    options
        .map(|options| {
            if bundle {
                Ok(Command::Bundle(options))
            } else {
                Ok(Command::Scene(options))
            }
        })
        .transpose()
}
pub fn run_bundle(options: Options) -> Result<serde_json::Value, String> {
    validate_options(&options)?;
    let root = crate::surface_bundle::io::checked_path(&options.scene)?;
    let capture = options.capture.as_deref().map(capture_path).transpose()?;
    if capture.as_ref().is_some_and(|p| p.starts_with(&root)) {
        return Err("capture output must be outside the bundle root".into());
    }
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let player = crate::surface_bundle::verify_bundle(&root, Some(&executable))?;
    run_player(options, player, capture)
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

fn validate_options(options: &Options) -> Result<(), String> {
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
    Ok(())
}

pub fn run(options: Options) -> Result<serde_json::Value, String> {
    validate_options(&options)?;
    // Refuse bad output before opening the scene or stepping authoritative state.
    let capture = options.capture.as_deref().map(capture_path).transpose()?;
    let player = SurfacePlayer::open(&options.scene)?;
    run_player(options, player, capture)
}

fn run_player(
    options: Options,
    mut player: SurfacePlayer,
    capture: Option<PathBuf>,
) -> Result<serde_json::Value, String> {
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
                capture_png(&path, &player)?
            }
            #[cfg(not(feature = "gpu"))]
            {
                let _ = path;
                return Err("capture requires gpu".into());
            }
        }
    };
    let result = serde_json::json!({
        "schema":2,"profile":"NavigationYard3DSurfacePointV2","surface_sha256":hex(player.surface().digest()),"tick":player.tick(),"checksum":player.checksum(),
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
fn capture_png(path: &Path, player: &SurfacePlayer) -> Result<serde_json::Value, String> {
    use orr_render::orr_rhi::{Rhi, Wgpu, WgpuOptions};
    use std::io::Write;
    let gpu = Wgpu::headless(WgpuOptions::default()).map_err(|e| e.to_string())?;
    let model = player.model()?;
    let size = (1000, 750);
    let camera = crate::gpu::framing::fitted_top_camera(&model, size).map_err(|e| e.to_string())?;
    let mut renderer = orr_render::ModelRenderer::new(gpu.clone(), crate::gpu::FORMAT, model)
        .map_err(|e| e.to_string())?;
    renderer.clear = [0.0, 0.0, 0.0, 1.0];
    let pixels = crate::gpu::capture(&mut renderer, &camera, &crate::gpu::lighting(), size)
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
