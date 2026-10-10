//! Standalone authored Arena execution. Admission and baking finish before any
//! thread, window or GPU is created. The editor is not a runtime dependency.
use crate::{
    arena_view::{arena_bridge_config, arena_floor, editor_drawables},
    project::{arena_types, PreparedProject, UiSupport},
    project_compositor::SpriteDraw,
    project_playback::{Observation, PlaybackState, Position, SnapshotPlayback},
    project_sprites::{Asset, AssetKey, Document},
};
use orr_bridge::{Bridge, InProc, PlayConfig, PlayHost, PlaySession, PlayerSlot, Snapshot};
use orr_ecs::{Entity, Frame};
use orr_fp::FrameRng;
use orr_reflect::{Guid, SceneIndex};
use orr_render::{camera::CameraFollow, extract_items, Camera, RenderList};
use orr_sim::Simulation;
use orr_testgame::Arena;
use orr_view::{RenderItem, Transform2, Vec2};
use std::{collections::BTreeMap, path::Path};

pub const SEED: u64 = 42;
pub const TICK_RATE: u32 = 60;
pub const PLAYERS: u8 = 2;

/// Existing local Arena editor-host identity, including its frame format binding.
/// Kept compatible with `orr_remote::default_build_id("Arena")` by parity tests;
/// this does not introduce a distinct game or claim an exported build identity.
pub fn build_id() -> u64 {
    let text = format!("orr_remote_host/{}/Arena", env!("CARGO_PKG_VERSION"));
    let id = text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    orr_sim::frame_build_id(id)
}

/// Only content capabilities actually consumed by this launch route.
/// Compiling unrelated optional sample features does not make them project entries.
pub fn compiled_runtime() -> orr_package::Runtime {
    let mut runtime = orr_package::Runtime::content_only();
    runtime.capabilities.insert("sprite".into());
    runtime
}

pub struct PreparedRuntime {
    project: PreparedProject,
    restart_seed: AuthoredRestartSeed,
    index: SceneIndex,
}

/// An immutable, admitted Arena launch seed.
///
/// Cloning this value preserves the exact scene baked at admission. Every
/// launch or restart makes a fresh `PlaySession` from this
/// same frame and config; it never consults project files or creates a bot.
#[derive(Clone)]
pub struct AuthoredRestartSeed {
    initial: Frame,
    config: PlayConfig,
}
impl AuthoredRestartSeed {
    fn new(initial: Frame) -> Self {
        let mut config = PlayConfig::new(PLAYERS, SEED, TICK_RATE);
        config.game_id = "Arena".into();
        config.build_id = build_id();
        config.start_paused = false;
        Self { initial, config }
    }

    pub fn initial_frame(&self) -> &Frame {
        &self.initial
    }

    /// The admitted fixed launch configuration, including the unchanged Arena
    /// build identity, seed, tick rate, and two human-controlled slots.
    pub fn config(&self) -> &PlayConfig {
        &self.config
    }

    pub fn session(&self) -> Result<PlaySession<Arena>, String> {
        PlaySession::from_frame(self.config.clone(), &self.initial)
            .map_err(|e| format!("Arena session: {e}"))
    }

    pub fn bridge(&self) -> Result<InProc<Arena, PlayHost<Arena>>, String> {
        Ok(InProc::new(
            PlayHost::new(self.session()?, PlayerSlot(0)),
            arena_bridge_config(),
        ))
    }
}
impl PreparedRuntime {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        let ui_support = if cfg!(feature = "game-ui") {
            UiSupport::Supported
        } else {
            UiSupport::Unsupported
        };
        let project = PreparedProject::open_with_ui(root, compiled_runtime(), ui_support)?;
        let registry = Simulation::<Arena>::build_registry();
        let mut initial = Frame::new(registry);
        initial.set_singleton(FrameRng::new(SEED));
        let index = project
            .scene()
            .scene()
            .bake(&arena_types(), &mut initial)
            .map_err(|e| format!("Arena scene bake: {e}"))?;
        Ok(Self {
            project,
            restart_seed: AuthoredRestartSeed::new(initial),
            index,
        })
    }
    pub fn initial_frame(&self) -> &Frame {
        self.restart_seed.initial_frame()
    }
    pub fn index(&self) -> &SceneIndex {
        &self.index
    }
    pub fn project(&self) -> &PreparedProject {
        &self.project
    }
    pub fn restart_seed(&self) -> &AuthoredRestartSeed {
        &self.restart_seed
    }
    /// Every new run starts from admitted bytes, with no automatic bot or live reread.
    pub fn session(&self) -> Result<PlaySession<Arena>, String> {
        self.restart_seed.session()
    }
    pub fn bridge(&self) -> Result<InProc<Arena, PlayHost<Arena>>, String> {
        self.restart_seed.bridge()
    }
    pub fn into_parts(self) -> Result<(PlaySession<Arena>, ProjectPresentation), String> {
        let (seed, presentation, ui) = self.into_launch_parts();
        if ui.is_some() {
            return Err(
                "authored project declares UI; use into_launch_parts for a UI-aware launch".into(),
            );
        }
        Ok((seed.session()?, presentation))
    }

    /// Consume the admitted launch package while retaining the immutable seed
    /// needed by app-level restart factories and the optional admitted UI font.
    pub fn into_launch_parts(
        self,
    ) -> (
        AuthoredRestartSeed,
        ProjectPresentation,
        Option<crate::project::PreparedUi>,
    ) {
        let (_, _, sprites, ui) = self.project.into_parts();
        let (document, assets) =
            sprites.map_or((None, BTreeMap::new()), |s| (Some(s.document), s.assets));
        (
            self.restart_seed,
            ProjectPresentation {
                #[cfg(feature = "collect-dodge")]
                collect: false,
                #[cfg(feature = "collect-dodge")]
                collect_camera_initialized: false,
                #[cfg(feature = "collect-dodge")]
                collect_elapsed: None,
                index: self.index,
                document,
                assets,
                sampler: SnapshotPlayback::default(),
                follow: CameraFollow::new(),
                camera: Camera::new([0.0, 0.0], 240.0),
                shapes: RenderList::new(),
                sprites: Vec::new(),
            },
            ui,
        )
    }
}

/// Shape underlays, sprite positions/cursors and follow all use one displayed
/// frame. No interpolation or wall-clock animation is mixed into this route.
pub struct ProjectPresentation {
    #[cfg(feature = "collect-dodge")]
    collect: bool,
    #[cfg(feature = "collect-dodge")]
    collect_camera_initialized: bool,
    #[cfg(feature = "collect-dodge")]
    collect_elapsed: Option<u32>,
    index: SceneIndex,
    document: Option<Document>,
    assets: BTreeMap<AssetKey, Asset>,
    sampler: SnapshotPlayback,
    follow: CameraFollow<Entity>,
    pub camera: Camera,
    shapes: RenderList,
    sprites: Vec<SpriteDraw>,
}
impl ProjectPresentation {
    #[cfg(feature = "collect-dodge")]
    pub(crate) fn collect(
        index: SceneIndex,
        sprites: Option<crate::project::PreparedSprites>,
    ) -> Self {
        let (document, assets) =
            sprites.map_or((None, BTreeMap::new()), |s| (Some(s.document), s.assets));
        Self {
            collect: true,
            collect_camera_initialized: false,
            collect_elapsed: None,
            index,
            document,
            assets,
            sampler: SnapshotPlayback::default(),
            follow: CameraFollow::new(),
            camera: Camera::new([0.0, 0.0], 270.0),
            shapes: RenderList::new(),
            sprites: Vec::new(),
        }
    }

    pub fn index(&self) -> &SceneIndex {
        &self.index
    }
    pub fn assets(&self) -> &BTreeMap<AssetKey, Asset> {
        &self.assets
    }
    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }
    pub fn shapes(&self) -> &RenderList {
        &self.shapes
    }
    pub fn sprites(&self) -> &[SpriteDraw] {
        &self.sprites
    }
    pub fn state(&self, guid: &str) -> PlaybackState {
        self.sampler.state(guid)
    }
    pub fn reset(&mut self) {
        #[cfg(feature = "collect-dodge")]
        {
            self.collect_camera_initialized = false;
            self.collect_elapsed = None;
        }
        self.sampler.reset();
        self.follow.stop();
        self.shapes.clear();
        self.sprites.clear();
        self.camera = Camera::new([0.0, 0.0], 240.0);
    }
    pub fn update(&mut self, snapshot: Option<&Snapshot>) -> Result<(), String> {
        self.shapes.clear();
        self.sprites.clear();
        let Some(snapshot) = snapshot else {
            self.sampler.reset();
            self.follow.stop();
            return Ok(());
        };
        let timeline = snapshot
            .timeline()
            .ok_or("authored project requires a Play timeline")?;
        if timeline.tick != snapshot.tick() || snapshot.tick_rate() == 0 {
            self.sampler.reset();
            self.follow.stop();
            return Err("authored project received an incoherent Play snapshot".into());
        }
        #[cfg(feature = "collect-dodge")]
        let bodies = if self.collect {
            let elapsed = snapshot
                .predicted()
                .singleton::<crate::collect_game::CollectRun>()
                .elapsed_ticks;
            if self
                .collect_elapsed
                .is_some_and(|previous| elapsed < previous)
            {
                self.sampler.reset();
            }
            self.collect_elapsed = Some(elapsed);
            if !self.collect_camera_initialized
                || self
                    .document
                    .as_ref()
                    .is_none_or(|document| document.camera_follow.is_none())
            {
                self.camera = crate::collect_view::scene_camera(snapshot.predicted());
                self.collect_camera_initialized = true;
            }
            crate::collect_view::editor_drawables(snapshot.predicted())
        } else {
            editor_drawables(snapshot.predicted())
        };
        #[cfg(not(feature = "collect-dodge"))]
        let bodies = editor_drawables(snapshot.predicted());
        #[cfg(feature = "collect-dodge")]
        let floor = if self.collect {
            crate::collect_view::scene_floor()
        } else {
            arena_floor()
        };
        #[cfg(not(feature = "collect-dodge"))]
        let floor = arena_floor();
        let items: Vec<_> = std::iter::once(floor)
            .chain(bodies.iter().map(|b| RenderItem {
                entity: b.entity,
                transform: Transform2::new(Vec2::new(b.pos[0], b.pos[1]), b.angle + b.turn),
                style: b.style,
            }))
            .collect();
        extract_items(&items, &mut self.shapes);
        let Some(document) = &self.document else {
            return Ok(());
        };
        // One generation-aware lookup index avoids a full drawable scan for
        // each admitted binding at the 4096-binding / 20000-entity ceiling.
        let by_entity: BTreeMap<_, _> = bodies.iter().map(|body| (body.entity, body)).collect();
        let positions: BTreeMap<_, _> = document
            .bindings
            .keys()
            .chain(document.camera_follow.iter())
            .map(|guid| {
                let entity = Guid::parse(guid).ok().and_then(|g| self.index.entity(&g));
                let position = entity
                    .and_then(|entity| by_entity.get(&entity).copied())
                    .filter(|b| b.pos.iter().all(|v| v.is_finite()))
                    .map(|b| Position {
                        entity: b.entity,
                        pos: b.pos,
                    });
                (guid.clone(), position)
            })
            .collect();
        self.sampler.observe(
            document,
            Observation {
                seq: snapshot.seq(),
                tick: timeline.tick,
                epoch: timeline.epoch,
                tick_rate: snapshot.tick_rate(),
                checksum: snapshot.predicted().checksum(),
                playing: timeline.playing,
            },
            &positions,
            &timeline.recent_checksums,
        );
        for (guid, binding) in &document.bindings {
            let Some(position) = positions.get(guid).copied().flatten() else {
                continue;
            };
            let key = (binding.package.clone(), binding.document.clone());
            let asset = self
                .assets
                .get(&key)
                .ok_or("admitted sprite asset is missing")?;
            let state = self.sampler.state(guid);
            let elapsed = state.elapsed_ms;
            #[cfg(feature = "collect-dodge")]
            let elapsed = if self.collect
                && !matches!(
                    binding.source,
                    crate::project_sprites::Source::Locomotion { .. }
                ) {
                u64::from(
                    snapshot
                        .predicted()
                        .singleton::<crate::collect_game::CollectRun>()
                        .elapsed_ticks,
                )
                .saturating_mul(1000)
                    / u64::from(snapshot.tick_rate())
            } else {
                elapsed
            };
            let id = binding.region_for_motion(&asset.document, elapsed, state.moving)?;
            let region = asset
                .document
                .region(id)
                .ok_or("admitted sprite region is missing")?;
            self.sprites.push(SpriteDraw {
                asset: key,
                instance: orr_sprite::SpriteInstance {
                    region: id,
                    position: position.pos,
                    size: [
                        region.width as f32 * binding.units_per_pixel,
                        region.height as f32 * binding.units_per_pixel,
                    ],
                    rotation: binding.orientation.unwrap_or_default().radians(),
                    flip_x: binding.orientation.unwrap_or_default().flip_x,
                    flip_y: binding.orientation.unwrap_or_default().flip_y,
                    ..Default::default()
                },
            });
        }
        if let Some(guid) = &document.camera_follow {
            let position = positions.get(guid).copied().flatten();
            if let Some(position) = position {
                self.follow.follow(position.entity);
            }
            self.follow.update(&mut self.camera, |entity| {
                position.filter(|p| p.entity == *entity).map(|p| p.pos)
            });
        }
        Ok(())
    }
}

/// A bounded deterministic smoke run of the actual admission/session/presentation
/// route. Capturing uses the same compositor as the project window.
pub fn headless(
    prepared: PreparedRuntime,
    ticks: u32,
    held: crate::arena_view::Keys,
    capture: Option<&Path>,
) -> Result<(), String> {
    if ticks > 6000 {
        return Err("--ticks must be between 0 and 6000".into());
    }
    let (seed, mut presentation, admitted_ui) = prepared.into_launch_parts();
    let session = seed.session()?;
    #[cfg(not(feature = "game-ui"))]
    drop(admitted_ui);
    let mut bridge = InProc::new(PlayHost::new(session, PlayerSlot(0)), arena_bridge_config());
    let initial = bridge
        .snapshot()
        .ok_or("initial project snapshot missing")?;
    println!(
        "project initial checksum: 0x{:016x}",
        initial.predicted().checksum()
    );
    presentation.update(Some(&initial))?;
    bridge
        .set_input(PlayerSlot(0), held.to_input())
        .map_err(|e| format!("project input: {e:?}"))?;
    for _ in 0..ticks {
        bridge.step(1);
        presentation.update(bridge.snapshot().as_ref())?;
    }
    let snapshot = bridge.snapshot().ok_or("project snapshot missing")?;
    println!(
        "project tick: {} checksum: 0x{:016x}",
        snapshot.tick(),
        snapshot.predicted().checksum()
    );
    for (guid, entity) in presentation.index().iter() {
        if let Some(pos) = snapshot.predicted().get::<orr_testgame::Position>(entity) {
            println!(
                "project entity: {guid} handle:{}v{} position:{},{}",
                entity.index,
                entity.version,
                pos.pos.x.raw(),
                pos.pos.y.raw()
            );
        }
    }
    if let Some(path) = capture {
        use orr_render::orr_rhi::{Rhi, TextureFormat, Wgpu, WgpuOptions};
        #[cfg(feature = "game-ui")]
        let mut game_ui = admitted_ui
            .map(|ui| crate::game_ui::GameUi::from_font(ui.font, true))
            .transpose()?;
        let gpu = Wgpu::headless(WgpuOptions::default())?;
        println!(
            "project capture adapter: {} (software: {})",
            gpu.adapter_name(),
            gpu.is_software()
        );
        let size = (512, 512);
        let target =
            orr_render::OffscreenTarget::new(&gpu, size.0, size.1, TextureFormat::Rgba8UnormSrgb);
        let mut compositor = crate::project_compositor::ProjectCompositor::new(
            gpu.clone(),
            TextureFormat::Rgba8UnormSrgb,
            presentation.assets(),
        )?;
        #[cfg(feature = "game-ui")]
        {
            let mut overlay = game_ui
                .as_ref()
                .map(|_| crate::game_ui_gpu::GpuOverlay::new(&gpu, TextureFormat::Rgba8UnormSrgb));
            let mut pending = crate::game_ui_gpu::PendingOverlay::default();
            compositor.draw_with_overlay(
                target.render_view(),
                size,
                presentation.shapes(),
                presentation.sprites(),
                &presentation.camera,
                |gpu, view, size| {
                    if let (Some(game_ui), Some(overlay)) = (game_ui.as_mut(), overlay.as_mut()) {
                        let screen = egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(size.0 as f32, size.1 as f32),
                        );
                        let hud = crate::game_ui::Hud {
                            tick: snapshot.tick(),
                            verified_tick: snapshot.tick(),
                            rollbacks: 0,
                        };
                        // Anchored egui windows need sizing passes before their
                        // button geometry settles. Keep every texture epoch so
                        // the final stable frame has its font atlas available.
                        for pass in 0..3 {
                            let input = egui::RawInput {
                                screen_rect: Some(screen),
                                // Fixed presentation-only time settles egui's opening
                                // fade without sleeping or advancing the simulation.
                                time: Some(f64::from(pass) * 0.5),
                                ..Default::default()
                            };
                            let (output, _) = game_ui.show(input, hud);
                            pending.push(output)?;
                        }
                        pending.paint(overlay, gpu, view, size, &game_ui.context)?;
                    }
                    Ok(())
                },
            )?;
        }
        #[cfg(not(feature = "game-ui"))]
        compositor.draw(
            target.render_view(),
            size,
            presentation.shapes(),
            presentation.sprites(),
            &presentation.camera,
        )?;
        let rgba = target.read_rgba8();
        let file = std::fs::File::create_new(path)
            .map_err(|e| format!("capture {}: {e}", path.display()))?;
        let mut encoder = png::Encoder::new(file, size.0, size.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(())
}
