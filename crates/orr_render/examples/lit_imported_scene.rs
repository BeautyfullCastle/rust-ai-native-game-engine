//! Package-backed static + animated scene, shared depth, and one editable point light.
//! See docs/imported-scene.md or run with --help. All floats are presentation-only.
#![allow(clippy::float_arithmetic, clippy::disallowed_types)]
use orr_model::{
    animation::{AnimatedModel, AnimationPlayer, PlaybackMode, PlaybackState},
    StaticModel, IDENTITY,
};
use orr_package::{Project, Runtime};
use orr_render::orr_rhi::{
    Acquire, Rhi, TextureDesc, TextureFormat, TextureUsage, Wgpu, WgpuOptions,
};
use orr_render::{
    Camera3D, ImportedBatch, ImportedSceneRenderer, ImportedSceneTarget, Lighting, ModelRenderer,
    PointLight, PointLightSettings, SkinnedInstance, SkinnedModelRenderer,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

const INSTALL: &str = "install both packages first: cargo run -p orr_package --bin orr_pkg -- install PROJECT --path assets/imported_scene_demo --path assets/animation_demo";
const HELP: &str = "Package-backed imported scene (static background, animated strip, static foreground).
Install from the repository root:
  mkdir -p /tmp/imported-scene-project
  cargo run -p orr_package --bin orr_pkg -- install /tmp/imported-scene-project --path assets/imported_scene_demo --path assets/animation_demo
Run:
  cargo run -p orr_render --features imported-scene,animation --example lit_imported_scene -- --project /tmp/imported-scene-project
Options:
  --project PATH         Installed content project (required)
  --offscreen PATH       Render one frame to an actual GPU-readback PPM; no window
  --size WIDTHxHEIGHT    Window/capture size, 1..8192 per axis (default 960x640)
  --seek SECONDS         Initial animation time, finite and nonnegative (default 0.5)
  --point on|off         Override the loaded point-light enable state
  --light-position X,Y,Z Override its world-space position (also enables the light)
  --settings PATH        User-owned light JSON (default PROJECT/lit-imported-scene-light.json)
  --save-settings        Save settings after applying CLI overrides
  --reverse-order       Submit foreground, animated, background instead
Controls: WASD move light in XY; Q/E move in Z; P toggle light; Space pause/resume;
Left/Right seek 0.1s; 1/2 select clip; F5 save; F9 reload; Escape exit.
F5/F9 use the user-owned settings file, never an installed package snapshot.
Missing settings start with the demo light on; the library default remains off.
See docs/imported-scene.md for the light equation and supported scope.";

#[derive(Clone)]
struct Assets {
    background: StaticModel,
    foreground: StaticModel,
    animated: AnimatedModel,
}
fn load_assets(root: &Path) -> Result<Assets, String> {
    let mut runtime = Runtime::content_only();
    runtime.capabilities.insert("models".into());
    runtime.capabilities.insert("animation".into());
    let project = Project::open(root, runtime).map_err(|e| format!("{e}; {INSTALL}"))?;
    let load_static = |asset: &str| -> Result<StaticModel, String> {
        let bytes = project
            .read_asset("sample-imported-scene", asset)
            .map_err(|e| format!("{e}; {INSTALL}"))?;
        let imported = orr_model::import::import_with_resolver(asset, &bytes, |uri| {
            project
                .read_asset("sample-imported-scene", uri)
                .map_err(|e| orr_model::Error::message(e.to_string()))
        })
        .map_err(|e| format!("import {asset}: {e}"))?;
        // Exercise the same immutable cooked reload used by runtime consumers.
        StaticModel::from_bytes(&imported.to_bytes().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    };
    let bytes = project
        .read_asset("sample-animation", "animated.glb")
        .map_err(|e| format!("{e}; {INSTALL}"))?;
    let imported =
        orr_model::animation_import::import_with_resolver("animated.glb", &bytes, |uri| {
            project
                .read_asset("sample-animation", uri)
                .map_err(|e| orr_model::Error::message(e.to_string()))
        })
        .map_err(|e| format!("import animated.glb: {e}"))?;
    Ok(Assets {
        background: load_static("background.glb")?,
        foreground: load_static("foreground.glb")?,
        animated: AnimatedModel::from_bytes(&imported.to_bytes().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?,
    })
}

fn camera() -> Camera3D {
    Camera3D::orthographic([3.0, 3.0, 9.0], [0.0, 1.5, 0.0], 2.5)
}
fn lighting() -> Lighting {
    Lighting {
        shadows: false,
        tonemap: false,
        ambient: 0.28,
        intensity: 0.18,
        sky: [0.85, 0.9, 1.0],
        ground: [0.6, 0.65, 0.75],
        ..Default::default()
    }
}
fn demo_light() -> PointLight {
    PointLight {
        position: [0.6, 2.2, 2.5],
        color: [1.0, 0.82, 0.58],
        intensity: 2.8,
        range: 6.0,
    }
}

struct Scene {
    rhi: Wgpu,
    composer: ImportedSceneRenderer<Wgpu>,
    background: ModelRenderer<Wgpu>,
    foreground: ModelRenderer<Wgpu>,
    animated: SkinnedModelRenderer<Wgpu>,
}
impl Scene {
    fn new(rhi: Wgpu, format: TextureFormat, assets: &Assets) -> Result<Self, String> {
        let mut composer =
            ImportedSceneRenderer::new(rhi.clone(), format).map_err(|e| e.to_string())?;
        composer.clear = [0.025, 0.035, 0.055, 1.0];
        Ok(Self {
            background: ModelRenderer::new(rhi.clone(), format, assets.background.clone())
                .map_err(|e| e.to_string())?,
            foreground: ModelRenderer::new(rhi.clone(), format, assets.foreground.clone())
                .map_err(|e| e.to_string())?,
            animated: SkinnedModelRenderer::new(rhi.clone(), format, assets.animated.clone())
                .map_err(|e| e.to_string())?,
            composer,
            rhi,
        })
    }
    // Both native presentation and headless readback call exactly this function.
    // No procedural RenderList3D or renderer-owned stand-in mesh is involved.
    fn draw(
        &mut self,
        view: &<Wgpu as Rhi>::TextureView,
        size: (u32, u32),
        player: &AnimationPlayer,
        point: &PointLightSettings,
        reverse: bool,
    ) -> Result<(), String> {
        let pose = player
            .pose(self.animated.model())
            .map_err(|e| e.to_string())?;
        let instances = [SkinnedInstance {
            pose: &pose,
            transform: IDENTITY,
        }];
        let mut batches = [
            ImportedBatch::Static(&mut self.background),
            ImportedBatch::Skinned {
                renderer: &mut self.animated,
                instances: &instances,
            },
            ImportedBatch::Static(&mut self.foreground),
        ];
        if reverse {
            batches.reverse();
        }
        self.composer
            .draw(
                ImportedSceneTarget {
                    view,
                    size,
                    format: self.composer.format(),
                    sample_count: 1,
                },
                &camera(),
                &lighting(),
                point,
                &mut batches,
            )
            .map_err(|e| e.to_string())
    }
}

// Resolve the parent first, so a user-supplied path cannot alias the package
// snapshot through a parent symlink. The file itself must not be a symlink.
// Recheck immediately before each save/reload, not only at startup.
fn user_settings_path(project: &Path, path: &Path) -> Result<PathBuf, String> {
    let project = project
        .canonicalize()
        .map_err(|e| format!("project path: {e}"))?;
    let file = path.file_name().ok_or("settings require a file name")?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let resolved = parent
        .canonicalize()
        .map_err(|e| format!("settings parent: {e}"))?
        .join(file);
    let package_state = project.join(".orr");
    let in_cache = resolved.starts_with(&package_state)
        || package_state
            .canonicalize()
            .is_ok_and(|p| resolved.starts_with(p))
        || resolved.components().any(|c| c.as_os_str() == ".orr");
    if in_cache {
        return Err("settings must be user-owned and outside .orr package snapshots".into());
    }
    match std::fs::symlink_metadata(&resolved) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
            return Err(
                "settings must be a regular user-owned file, not a symlink or directory".into(),
            );
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("settings path: {e}")),
    }
    Ok(resolved)
}

struct Controls {
    player: AnimationPlayer,
    point: PointLightSettings,
    remembered: PointLight,
    project: PathBuf,
    settings: PathBuf,
    reverse: bool,
}
impl Controls {
    fn toggle_light(&mut self) {
        if let Some(light) = self.point.point_light.take() {
            self.remembered = light;
        } else {
            self.point.point_light = Some(self.remembered);
        }
    }
    fn move_light(&mut self, axis: usize, delta: f32) {
        let light = self.point.point_light.get_or_insert(self.remembered);
        // Bounded keyboard motion remains inside the validated presentation range.
        light.position[axis] = (light.position[axis] + delta).clamp(-100.0, 100.0);
        self.remembered = *light;
    }
    fn save(&self) -> Result<(), String> {
        let path = user_settings_path(&self.project, &self.settings)?;
        self.point
            .save(&path)
            .map_err(|e| format!("save {}: {e}", path.display()))?;
        eprintln!("saved user light settings: {}", path.display());
        Ok(())
    }
    fn reload(&mut self) -> Result<(), String> {
        let path = user_settings_path(&self.project, &self.settings)?;
        // PointLightSettings::reload validates a complete replacement first.
        self.point
            .reload(&path)
            .map_err(|e| format!("reload {}: {e}; active light unchanged", path.display()))?;
        if let Some(light) = &self.point.point_light {
            self.remembered = *light;
        }
        eprintln!("reloaded user light settings: {}", path.display());
        Ok(())
    }
    fn key(&mut self, key: KeyCode, model: &AnimatedModel) -> Result<(), String> {
        match key {
            KeyCode::KeyP => self.toggle_light(),
            KeyCode::KeyA => self.move_light(0, -0.25),
            KeyCode::KeyD => self.move_light(0, 0.25),
            KeyCode::KeyW => self.move_light(1, 0.25),
            KeyCode::KeyS => self.move_light(1, -0.25),
            KeyCode::KeyQ => self.move_light(2, -0.25),
            KeyCode::KeyE => self.move_light(2, 0.25),
            KeyCode::Space => {
                if self.player.state() == PlaybackState::Playing {
                    self.player.pause();
                } else {
                    self.player.resume();
                }
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let delta = if key == KeyCode::ArrowLeft { -0.1 } else { 0.1 };
                self.player
                    .seek(model, (self.player.time() + delta).max(0.0))
                    .map_err(|e| e.to_string())?;
            }
            KeyCode::Digit1 | KeyCode::Digit2 => {
                let clip = if key == KeyCode::Digit1 { 0 } else { 1 };
                self.player
                    .play(model, clip, PlaybackMode::Loop)
                    .map_err(|e| e.to_string())?;
            }
            KeyCode::F5 => self.save()?,
            KeyCode::F9 => self.reload()?,
            _ => {}
        }
        Ok(())
    }
}

struct Gfx {
    window: Arc<Window>,
    surface: <Wgpu as Rhi>::Surface,
    scene: Scene,
}
struct App {
    assets: Assets,
    controls: Controls,
    size: (u32, u32),
    gfx: Option<Gfx>,
    last: Instant,
    error: Option<String>,
}
impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl ToString) {
        self.error = Some(error.to_string());
        event_loop.exit();
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let result = (|| -> Result<Gfx, String> {
            let window = Arc::new(event_loop.create_window(
                Window::default_attributes()
                    .with_title("Orrery imported scene | WASDQE light | P on/off | arrows seek | F5 save | F9 reload")
                    .with_inner_size(winit::dpi::PhysicalSize::new(self.size.0, self.size.1)),
            ).map_err(|e| e.to_string())?);
            let size = window.inner_size();
            let (rhi, surface) = Wgpu::for_window(
                window.clone(),
                (size.width.max(1), size.height.max(1)),
                true,
                WgpuOptions::default(),
            )?;
            let scene = Scene::new(rhi.clone(), rhi.surface_format(&surface), &self.assets)?;
            Ok(Gfx {
                window,
                surface,
                scene,
            })
        })();
        match result {
            Ok(gfx) => {
                self.gfx = Some(gfx);
                self.last = Instant::now();
            }
            Err(e) => self.fail(event_loop, e),
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(g) = &mut self.gfx {
                    if size.width > 0 && size.height > 0 {
                        g.scene
                            .rhi
                            .resize_surface(&mut g.surface, size.width, size.height);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                if let PhysicalKey::Code(key) = event.physical_key {
                    if key == KeyCode::Escape {
                        event_loop.exit();
                    } else if let Err(e) = self.controls.key(key, &self.assets.animated) {
                        // Failed saves/reloads are recoverable; keep rendering the last valid state.
                        eprintln!("lit_imported_scene: {e}");
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last).as_secs_f32().min(0.1);
                self.last = now;
                if let Err(e) = self.controls.player.advance(&self.assets.animated, dt) {
                    self.fail(event_loop, e);
                    return;
                }
                if let Some(g) = &mut self.gfx {
                    let size = g.window.inner_size();
                    if size.width == 0 || size.height == 0 {
                        return;
                    }
                    if let Acquire::Frame(frame) = g.scene.rhi.acquire_frame(&mut g.surface) {
                        let view = g.scene.rhi.frame_view(&frame).clone();
                        if let Err(e) = g.scene.draw(
                            &view,
                            (size.width, size.height),
                            &self.controls.player,
                            &self.controls.point,
                            self.controls.reverse,
                        ) {
                            self.fail(event_loop, e);
                            return;
                        }
                        g.scene.rhi.present(frame);
                    }
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(g) = &self.gfx {
            g.window.request_redraw();
        }
    }
}

fn parse_size(value: &str) -> Result<(u32, u32), String> {
    let (width, height) = value
        .split_once('x')
        .ok_or("--size requires WIDTHxHEIGHT")?;
    let width: u32 = width.parse().map_err(|_| "invalid capture width")?;
    let height: u32 = height.parse().map_err(|_| "invalid capture height")?;
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err("size must be in 1..8192 per axis".into());
    }
    Ok((width, height))
}
fn parse_position(value: &str) -> Result<[f32; 3], String> {
    let numbers: Vec<f32> = value
        .split(',')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| "--light-position requires three finite numbers X,Y,Z")?;
    let position: [f32; 3] = numbers
        .try_into()
        .map_err(|_| "--light-position requires X,Y,Z")?;
    if position.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
        return Err("--light-position requires finite values within [-1e9, 1e9]".into());
    }
    Ok(position)
}
fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut project = None;
    let mut output = None;
    let mut settings = None;
    let mut size = (960, 640);
    let mut seek = 0.5_f32;
    let mut enabled = None;
    let mut position = None;
    let mut save = false;
    let mut reverse = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" => {
                println!("{HELP}");
                return Ok(());
            }
            "--project" => {
                project = Some(PathBuf::from(args.next().ok_or("missing project path")?))
            }
            "--offscreen" => output = Some(PathBuf::from(args.next().ok_or("missing PPM path")?)),
            "--settings" => {
                settings = Some(PathBuf::from(args.next().ok_or("missing settings path")?))
            }
            "--size" => size = parse_size(&args.next().ok_or("missing size")?)?,
            "--seek" => {
                seek = args
                    .next()
                    .ok_or("missing animation time")?
                    .parse()
                    .map_err(|_| "invalid animation time")?;
                if !seek.is_finite() || seek < 0.0 {
                    return Err("--seek must be finite and nonnegative".into());
                }
            }
            "--point" => {
                enabled = Some(match args.next().as_deref() {
                    Some("on") => true,
                    Some("off") => false,
                    _ => return Err("--point requires on or off".into()),
                })
            }
            "--light-position" => {
                position = Some(parse_position(
                    &args.next().ok_or("missing light position")?,
                )?)
            }
            "--save-settings" => save = true,
            "--reverse-order" => reverse = true,
            _ => return Err(format!("unknown argument {arg}; see --help")),
        }
    }
    let project = project.ok_or("--project path required; see --help")?;
    let assets = load_assets(&project)?;
    let settings = user_settings_path(
        &project,
        &settings.unwrap_or_else(|| project.join("lit-imported-scene-light.json")),
    )?;
    let mut point = if settings.try_exists().map_err(|e| e.to_string())? {
        PointLightSettings::load(&settings)
            .map_err(|e| format!("load {}: {e}", settings.display()))?
    } else {
        PointLightSettings {
            point_light: Some(demo_light()),
        }
    };
    if let Some(position) = position {
        point.point_light.get_or_insert_with(demo_light).position = position;
    }
    let remembered = point.point_light.unwrap_or_else(demo_light);
    if let Some(enabled) = enabled {
        point.point_light = enabled.then_some(remembered);
    }
    point.validate().map_err(|e| e.to_string())?;
    let mut player = AnimationPlayer::default();
    player
        .play(&assets.animated, 0, PlaybackMode::Loop)
        .map_err(|e| e.to_string())?;
    player
        .seek(&assets.animated, seek)
        .map_err(|e| e.to_string())?;
    let controls = Controls {
        player,
        point,
        remembered,
        project,
        settings,
        reverse,
    };
    if save {
        controls.save()?;
    }
    eprintln!("user light settings: {}", controls.settings.display());
    if let Some(output) = output {
        let rhi = Wgpu::headless(WgpuOptions::default())?;
        eprintln!(
            "adapter: {} (software: {})",
            rhi.adapter_name(),
            rhi.is_software()
        );
        // A single render/readback view also works on downlevel adapters that
        // lack VIEW_FORMATS. The editor's dual-view OffscreenTarget is not needed.
        let format = TextureFormat::Rgba8UnormSrgb;
        let target = rhi.create_texture(&TextureDesc {
            label: "lit imported scene capture",
            width: size.0,
            height: size.1,
            format,
            usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::COPY_SRC,
            sample_count: 1,
            view_formats: &[],
        });
        let view = rhi.create_texture_view(&target, None);
        let mut scene = Scene::new(rhi.clone(), format, &assets)?;
        scene.draw(
            &view,
            size,
            &controls.player,
            &controls.point,
            controls.reverse,
        )?;
        let pixels = rhi.read_texture(&target);
        let mut ppm = format!("P6\n{} {}\n255\n", size.0, size.1).into_bytes();
        for p in pixels.chunks_exact(4) {
            ppm.extend_from_slice(&p[..3]);
        }
        std::fs::write(&output, ppm).map_err(|e| format!("write {}: {e}", output.display()))?;
        eprintln!("captured {}x{} to {}", size.0, size.1, output.display());
        return Ok(());
    }
    let mut app = App {
        assets,
        controls,
        size,
        gfx: None,
        last: Instant::now(),
        error: None,
    };
    EventLoop::new()
        .map_err(|e| e.to_string())?
        .run_app(&mut app)
        .map_err(|e| e.to_string())?;
    if let Some(error) = app.error {
        return Err(error);
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("lit_imported_scene: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "orr-lit-scene-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn installed_assets_import_and_cook_without_fallbacks() {
        let root = Temp::new();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets")
            .canonicalize()
            .unwrap();
        let project =
            Project::open_for_install(&root.0, Runtime::content_only().engine_version).unwrap();
        project
            .install(&[
                source.join("imported_scene_demo"),
                source.join("animation_demo"),
            ])
            .unwrap();
        let assets = load_assets(&root.0).unwrap();
        assert!(!assets.background.source().primitives.is_empty());
        assert!(!assets.foreground.source().primitives.is_empty());
        let mut player = AnimationPlayer::default();
        player
            .play(&assets.animated, 0, PlaybackMode::Loop)
            .unwrap();
        player.seek(&assets.animated, 0.5).unwrap();
        player.pose(&assets.animated).unwrap();
        let mut controls = Controls {
            player,
            point: PointLightSettings {
                point_light: Some(demo_light()),
            },
            remembered: demo_light(),
            project: root.0.clone(),
            settings: root.0.join("light.json"),
            reverse: false,
        };
        controls.move_light(0, 0.75);
        let saved = controls.point.clone();
        controls.save().unwrap();
        controls.toggle_light();
        assert!(controls.point.point_light.is_none());
        controls.reload().unwrap();
        assert_eq!(controls.point, saved);
        std::fs::write(&controls.settings, br#"{"version":999,"point_light":null}"#).unwrap();
        assert!(controls.reload().is_err());
        assert_eq!(controls.point, saved);
        // User-owned settings edits never invalidate immutable package digests.
        project.verify().unwrap();
    }
    #[test]
    fn missing_package_explains_installation() {
        let root = Temp::new();
        let error = match load_assets(&root.0) {
            Ok(_) => panic!("missing packages loaded"),
            Err(e) => e,
        };
        assert!(error.contains("install both packages first"), "{error}");
    }
    #[test]
    fn settings_cannot_target_installed_package_objects() {
        let root = Temp::new();
        let object = root.0.join(".orr/packages/objects/example");
        std::fs::create_dir_all(&object).unwrap();
        assert!(user_settings_path(&root.0, &object.join("light.json")).is_err());
        assert!(user_settings_path(&root.0, &root.0.join("light.json")).is_ok());
    }
    #[cfg(unix)]
    #[test]
    fn settings_cannot_alias_package_files_through_links() {
        use std::os::unix::fs::symlink;
        let root = Temp::new();
        let object = root.0.join(".orr/packages/objects/example");
        std::fs::create_dir_all(&object).unwrap();
        std::fs::write(object.join("light.json"), b"{}").unwrap();
        symlink(&object, root.0.join("alias")).unwrap();
        symlink(object.join("light.json"), root.0.join("light.json")).unwrap();
        assert!(user_settings_path(&root.0, &root.0.join("alias/light.json")).is_err());
        assert!(user_settings_path(&root.0, &root.0.join("light.json")).is_err());
    }
    #[test]
    fn capture_arguments_reject_invalid_values() {
        assert_eq!(parse_size("960x640").unwrap(), (960, 640));
        for value in ["0x640", "8193x640", "640", "-1x640"] {
            assert!(parse_size(value).is_err());
        }
        assert_eq!(parse_position("1,2,3").unwrap(), [1.0, 2.0, 3.0]);
        assert_eq!(parse_position("-1e9,0,1e9").unwrap(), [-1e9, 0.0, 1e9]);
        // Reject before --point off can hide an invalid remembered position.
        for value in [
            "1,2",
            "1,2,3,4",
            "NaN,2,3",
            "inf,2,3",
            "1e10,2,3",
            "-1e10,2,3",
        ] {
            assert!(parse_position(value).is_err());
        }
    }
}
