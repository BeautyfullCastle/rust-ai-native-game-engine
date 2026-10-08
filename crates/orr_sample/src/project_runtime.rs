//! Standalone authored Arena execution. Admission and baking finish before any
//! thread, window or GPU is created. The editor is not a runtime dependency.
use crate::{
    arena_view::{arena_bridge_config, arena_floor, editor_drawables},
    project::{arena_types, PreparedProject},
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
    initial: Frame,
    index: SceneIndex,
}
impl PreparedRuntime {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        let project = PreparedProject::open(root, compiled_runtime())?;
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
            initial,
            index,
        })
    }
    pub fn initial_frame(&self) -> &Frame {
        &self.initial
    }
    pub fn index(&self) -> &SceneIndex {
        &self.index
    }
    pub fn project(&self) -> &PreparedProject {
        &self.project
    }
    /// Every new run starts from admitted bytes, with no automatic bot or live reread.
    pub fn session(&self) -> Result<PlaySession<Arena>, String> {
        let mut cfg = PlayConfig::new(PLAYERS, SEED, TICK_RATE);
        cfg.game_id = "Arena".into();
        cfg.build_id = build_id();
        cfg.start_paused = false;
        PlaySession::from_frame(cfg, &self.initial).map_err(|e| format!("Arena session: {e}"))
    }
    pub fn bridge(&self) -> Result<InProc<Arena, PlayHost<Arena>>, String> {
        Ok(InProc::new(
            PlayHost::new(self.session()?, PlayerSlot(0)),
            arena_bridge_config(),
        ))
    }
    pub fn into_parts(self) -> Result<(PlaySession<Arena>, ProjectPresentation), String> {
        let session = self.session()?;
        let (_, _, sprites) = self.project.into_parts();
        let (document, assets) =
            sprites.map_or((None, BTreeMap::new()), |s| (Some(s.document), s.assets));
        Ok((
            session,
            ProjectPresentation {
                index: self.index,
                document,
                assets,
                sampler: SnapshotPlayback::default(),
                follow: CameraFollow::new(),
                camera: Camera::new([0.0, 0.0], 240.0),
                shapes: RenderList::new(),
                sprites: Vec::new(),
            },
        ))
    }
}

/// Shape underlays, sprite positions/cursors and follow all use one displayed
/// frame. No interpolation or wall-clock animation is mixed into this route.
pub struct ProjectPresentation {
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
        let bodies = editor_drawables(snapshot.predicted());
        let items: Vec<_> = std::iter::once(arena_floor())
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
            let id = binding.region_for_motion(&asset.document, state.elapsed_ms, state.moving)?;
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
    let (session, mut presentation) = prepared.into_parts()?;
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
