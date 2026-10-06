//! Optional package-backed Arena presentation. No simulation state is changed.
//! The binding follows the current local player's generation-aware entity handle;
//! it is deliberately not an editor-persistent scene binding.
use std::{
    io::{BufReader, Cursor},
    path::Path,
    sync::Arc,
    time::Duration,
};

use orr_bridge::Snapshot;
use orr_ecs::Entity;
use orr_package::{Project, Runtime};
use orr_render::orr_rhi::{Acquire, Rhi, Wgpu, WgpuOptions};
use orr_render::{
    camera::CameraFollow, Camera, RenderList, Renderer, SpriteDrawList, SpriteRenderer,
};
use orr_sprite::{Playback, SpriteDocument, SpriteInstance};
use orr_testgame::PlayerTag;
use orr_view::RenderItem;
use winit::window::Window;

const PACKAGE: &str = "sample-sprites";

pub struct SpriteScene {
    document: SpriteDocument,
    rgba: Vec<u8>,
    pub camera: Camera,
    follow: CameraFollow<Entity>,
    playback: Playback,
    previous: Option<(Entity, [f32; 2])>,
    moving: bool,
    since_movement: Duration,
    facing_left: bool,
    elapsed_remainder: Duration,
    pub list: SpriteDrawList,
}

impl SpriteScene {
    /// Opens only installed, hash-verified assets. There is no embedded fallback.
    pub fn open(root: &Path) -> Result<Self, String> {
        let mut runtime = Runtime::content_only();
        // This module is compiled only with `sprites`; do not trust JSON claims.
        runtime.capabilities.insert("sprite".into());
        let project = Project::open(root, runtime)
            .map_err(|e| format!("sprite project {}: {e}", root.display()))?;
        let bytes = project.read_asset(PACKAGE, "sprites.json").map_err(|e| {
            format!(
                "sprite package '{PACKAGE}': {e}; install assets/sprite_demo with orr_pkg first"
            )
        })?;
        let json = std::str::from_utf8(&bytes).map_err(|e| format!("sprite JSON UTF-8: {e}"))?;
        let document =
            SpriteDocument::from_json(json).map_err(|e| format!("sprite document: {e}"))?;
        for id in ["idle", "walk"] {
            if document.clip(id).is_none() {
                return Err(format!("sprite package requires clip '{id}'"));
            }
        }
        let png = project
            .read_asset(PACKAGE, &document.atlas().image)
            .map_err(|e| format!("sprite atlas: {e}"))?;
        let rgba = decode_atlas(&png, &document)?;
        Ok(Self {
            document,
            rgba,
            camera: Camera::new([0.0, 0.0], 500.0),
            follow: CameraFollow::new(),
            playback: Playback::default(),
            previous: None,
            moving: false,
            since_movement: Duration::MAX,
            facing_left: false,
            elapsed_remainder: Duration::ZERO,
            list: SpriteDrawList::default(),
        })
    }

    /// Uses the same interpolated/rollback-smoothed positions as shape rendering.
    /// A resync clears presentation history, without writing into the simulation.
    pub fn update(
        &mut self,
        dt: Duration,
        snapshot: Option<&Snapshot>,
        local_slot: u8,
        reset: bool,
        items: &mut Vec<RenderItem>,
    ) {
        if reset {
            self.follow.stop();
            self.previous = None;
            self.since_movement = Duration::MAX;
            self.playback.reset();
            self.elapsed_remainder = Duration::ZERO;
        }
        let target = snapshot.and_then(|s| {
            s.predicted()
                .iter::<PlayerTag>()
                .find(|(_, tag)| tag.slot == u32::from(local_slot))
                .map(|(entity, _)| entity)
        });
        if self.follow.target().copied() != target {
            self.follow.stop();
            if let Some(entity) = target {
                self.follow.follow(entity);
            }
            self.previous = None;
            self.since_movement = Duration::MAX;
            self.playback.reset();
            self.elapsed_remainder = Duration::ZERO;
        }
        self.list.clear();
        let item =
            target.and_then(|entity| items.iter().find(|item| item.entity == entity).copied());
        let Some(item) = item else {
            self.previous = None;
            return;
        };
        let position = [item.transform.pos.x, item.transform.pos.y];
        let delta = self
            .previous
            .filter(|(entity, _)| *entity == item.entity)
            .map(|(_, previous)| [position[0] - previous[0], position[1] - previous[1]])
            .unwrap_or([0.0; 2]);
        let displaced = delta[0].abs() + delta[1].abs() > 0.001;
        self.since_movement = if displaced {
            Duration::ZERO
        } else {
            self.since_movement.saturating_add(dt)
        };
        // Render frames can outnumber sim ticks; avoid resetting the walk clip
        // on every repeated snapshot between two published positions.
        let moving = self.since_movement < Duration::from_millis(100);
        if moving != self.moving {
            self.playback.reset();
            self.elapsed_remainder = Duration::ZERO;
        }
        self.moving = moving;
        if delta[0].abs() > 0.001 {
            self.facing_left = delta[0] < 0.0;
        }
        self.previous = Some((item.entity, position));
        self.elapsed_remainder = self.elapsed_remainder.saturating_add(dt);
        let millis = self.elapsed_remainder.as_millis().min(u128::from(u64::MAX)) as u64;
        self.elapsed_remainder = self
            .elapsed_remainder
            .saturating_sub(Duration::from_millis(millis));
        self.playback.advance(millis);
        let clip = self
            .document
            .clip(if self.moving { "walk" } else { "idle" })
            .expect("validated clips");
        self.list.push(SpriteInstance {
            region: self.playback.sample(clip).region,
            position,
            size: [100.0; 2],
            flip_x: self.facing_left,
            ..Default::default()
        });
        self.follow.update(&mut self.camera, |entity| {
            (*entity == item.entity).then_some(position)
        });
        // The floor, other player and bullets remain on the existing shape path.
        items.retain(|other| other.entity != item.entity);
    }
}

fn decode_atlas(bytes: &[u8], document: &SpriteDocument) -> Result<Vec<u8>, String> {
    let atlas = document.atlas();
    if atlas.width > 2048 || atlas.height > 2048 {
        return Err("sprite sample atlas dimensions exceed 2048 pixels".into());
    }
    let mut decoder = png::Decoder::new(BufReader::new(Cursor::new(bytes)));
    decoder.set_limits(png::Limits {
        bytes: 16 * 1024 * 1024,
    });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let header = decoder
        .read_header_info()
        .map_err(|e| format!("sprite PNG: {e}"))?;
    // Reject encoded dimensions before initializing image buffers or reading IDAT.
    if header.width > 2048 || header.height > 2048 {
        return Err("sprite PNG dimensions exceed 2048 pixels".into());
    }
    if header.width != atlas.width || header.height != atlas.height {
        return Err("sprite PNG dimensions differ from atlas document".into());
    }
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("sprite PNG: {e}"))?;
    if reader.info().animation_control.is_some() {
        return Err("sprite sample requires a static PNG atlas".into());
    }
    let len = reader.output_buffer_size().ok_or("sprite PNG too large")?;
    if len > 16 * 1024 * 1024 {
        return Err("sprite PNG exceeds 16 MiB decode limit".into());
    }
    let mut output = vec![0; len];
    let info = reader
        .next_frame(&mut output)
        .map_err(|e| format!("sprite PNG: {e}"))?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err("sprite sample requires RGBA8 PNG".into());
    }
    let expected_len = (atlas.width as usize)
        .checked_mul(atlas.height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("sprite PNG too large")?;
    if info.width != atlas.width
        || info.height != atlas.height
        || info.buffer_size() != expected_len
    {
        return Err("sprite PNG decoded frame differs from atlas document".into());
    }
    output.truncate(expected_len);
    Ok(output)
}

/// Sample-specific surface composition, keeping the core renderer API unchanged.
pub struct SpriteWindow {
    rhi: Wgpu,
    surface: <Wgpu as Rhi>::Surface,
    size: (u32, u32),
    shapes: Renderer<Wgpu>,
    sprites: SpriteRenderer<Wgpu>,
}
impl SpriteWindow {
    pub fn new(window: Arc<Window>, vsync: bool, scene: &SpriteScene) -> Result<Self, String> {
        let size = (
            window.inner_size().width.max(1),
            window.inner_size().height.max(1),
        );
        let (rhi, surface) = Wgpu::for_window(
            window,
            size,
            vsync,
            WgpuOptions {
                allow_software_fallback: false,
                ..Default::default()
            },
        )?;
        let format = rhi.surface_format(&surface);
        let shapes = Renderer::new(rhi.clone(), format);
        let mut sprites =
            SpriteRenderer::new(rhi.clone(), format, scene.document.clone(), &scene.rgba)
                .map_err(|e| e.to_string())?;
        sprites.clear = None;
        Ok(Self {
            rhi,
            surface,
            size,
            shapes,
            sprites,
        })
    }
    pub fn adapter_name(&self) -> String {
        self.rhi.adapter_name()
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        if width != 0 && height != 0 {
            self.size = (width, height);
            self.rhi.resize_surface(&mut self.surface, width, height);
        }
    }
    pub fn render(&mut self, list: &RenderList, scene: &SpriteScene) -> Result<(), String> {
        let Acquire::Frame(frame) = self.rhi.acquire_frame(&mut self.surface) else {
            return Ok(());
        };
        let view = self.rhi.frame_view(&frame);
        self.shapes.draw(view, self.size, list, &scene.camera);
        self.sprites
            .draw(view, self.size, &scene.list, &scene.camera)
            .map_err(|e| e.to_string())?;
        self.rhi.present(frame);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena_view::{arena_bridge_config, loopback_pair, ArenaExtractor, Keys, Loopback};
    use orr_bridge::{Bridge, InProc};
    use orr_view::{InterpMode, ViewConfig, ViewWorld};
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "orr-sprite-scene-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn install(&self) -> Project {
            let mut runtime = Runtime::content_only();
            runtime.capabilities.insert("sprite".into());
            let project = Project::open(&self.0, runtime).unwrap();
            project
                .install(&[PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("assets/sprite_demo")])
                .unwrap();
            project
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn installed_package_is_required_and_removal_is_visible() {
        let fixture = Fixture::new();
        assert!(SpriteScene::open(&fixture.0)
            .err()
            .unwrap()
            .contains("sample-sprites"));
        let project = fixture.install();
        let scene = SpriteScene::open(&fixture.0).unwrap();
        assert_eq!(scene.rgba.len(), 64 * 16 * 4);
        project.remove(PACKAGE).unwrap();
        assert!(SpriteScene::open(&fixture.0).is_err());
    }

    #[test]
    fn installed_png_hash_is_checked() {
        let fixture = Fixture::new();
        let project = fixture.install();
        let lock = project.list().unwrap();
        let package = &lock.packages[PACKAGE];
        let atlas = fixture
            .0
            .join(".orr/packages/objects")
            .join(&package.digest)
            .join("lantern_keeper.png");
        fs::write(atlas, b"tampered").unwrap();
        assert!(SpriteScene::open(&fixture.0).is_err());
    }

    #[test]
    fn actual_arena_moves_animates_and_follows_without_changing_checksums() {
        let fixture = Fixture::new();
        fixture.install();
        let mut scene = SpriteScene::open(&fixture.0).unwrap();
        let mut bridge = InProc::new(loopback_pair(Loopback::default()), arena_bridge_config());
        let mut baseline = InProc::new(loopback_pair(Loopback::default()), arena_bridge_config());
        let mut view = ViewWorld::new(
            ArenaExtractor {
                remote_mode: InterpMode::Snapshot,
                local_slot: 0,
            },
            ViewConfig::default(),
        );
        let mut saw_idle = false;
        let mut saw_walk = false;
        let mut saw_camera_move = false;
        for step in 0..100 {
            let dt = Duration::from_nanos(16_666_667);
            let keys = Keys {
                right: (20..70).contains(&step),
                ..Default::default()
            };
            bridge
                .set_input(bridge.local_slot(), keys.to_input())
                .unwrap();
            baseline
                .set_input(baseline.local_slot(), keys.to_input())
                .unwrap();
            bridge.update(dt);
            baseline.update(dt);
            let update = bridge.poll_view();
            let control = baseline.poll_view();
            view.update_from_bridge(dt.as_secs_f32(), &update);
            let mut items = Vec::new();
            view.render_items(&mut items);
            scene.update(
                dt,
                update.snapshot.as_ref(),
                0,
                update.resync.is_some(),
                &mut items,
            );
            if let Some(sprite) = scene.list.sprites.first() {
                saw_idle |= matches!(sprite.region, 10 | 11);
                saw_walk |= matches!(sprite.region, 20 | 21);
                assert_eq!(scene.camera.center, sprite.position);
                saw_camera_move |= step > 30 && scene.moving;
                assert!(!items
                    .iter()
                    .any(|item| Some(&item.entity) == scene.follow.target()));
            }
            assert_eq!(
                update.snapshot.as_ref().map(|s| s.predicted().checksum()),
                control.snapshot.as_ref().map(|s| s.predicted().checksum())
            );
        }
        assert!(saw_idle && saw_walk && saw_camera_move);
        scene.update(Duration::ZERO, None, 0, true, &mut Vec::new());
        assert!(scene.list.sprites.is_empty());
        assert!(scene.follow.target().is_none());
    }

    fn header_png(width: u32, height: u32) -> Vec<u8> {
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            let mut crc = u32::MAX;
            for byte in kind.iter().chain(data) {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
                }
            }
            out.extend_from_slice(&(!crc).to_be_bytes());
        }
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(&mut bytes, b"IHDR", &ihdr);
        chunk(&mut bytes, b"IDAT", &[]);
        chunk(&mut bytes, b"IEND", &[]);
        bytes
    }

    fn empty_document(width: u32, height: u32) -> SpriteDocument {
        SpriteDocument::new(orr_sprite::SpriteSource {
            format: orr_sprite::FORMAT.into(),
            version: orr_sprite::VERSION,
            atlas: orr_sprite::Atlas {
                image: "test.png".into(),
                width,
                height,
            },
            regions: Vec::new(),
            clips: Vec::new(),
        })
        .unwrap()
    }

    #[test]
    fn png_malformed_mismatch_and_crc_valid_dimension_boundary() {
        let doc = empty_document(2048, 1);
        assert!(decode_atlas(b"not a PNG", &doc)
            .unwrap_err()
            .starts_with("sprite PNG:"));
        // Valid CRC and PNG headers reach our dimension guard before any pixel
        // allocation or IDAT decode. The exact limit passes that guard and
        // reaches the expected empty-IDAT error instead.
        let over = decode_atlas(&header_png(2049, 1), &doc).unwrap_err();
        assert_eq!(over, "sprite PNG dimensions exceed 2048 pixels");
        assert_eq!(decode_atlas(&header_png(1, 2049), &doc).unwrap_err(), over);
        assert_eq!(
            decode_atlas(&header_png(2047, 1), &doc).unwrap_err(),
            "sprite PNG dimensions differ from atlas document"
        );
        let at_limit = decode_atlas(&header_png(2048, 1), &doc).unwrap_err();
        assert!(at_limit.starts_with("sprite PNG:"), "{at_limit}");
    }

    #[test]
    fn animated_atlas_is_rejected_before_renderer_creation() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_animated(1, 0).unwrap();
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[255, 0, 0, 255].repeat(4))
                .unwrap();
        }
        assert_eq!(
            decode_atlas(&bytes, &empty_document(2, 2)).unwrap_err(),
            "sprite sample requires a static PNG atlas"
        );
    }

    #[test]
    fn malformed_smaller_apng_frame_is_rejected() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_animated(1, 0).unwrap();
            let mut writer = encoder.write_header().unwrap();
            writer.set_frame_dimension(1, 1).unwrap();
            writer.write_image_data(&[255, 0, 0, 255]).unwrap();
        }
        assert!(decode_atlas(&bytes, &empty_document(2, 2)).is_err());
    }

    fn snapshot(frame: orr_ecs::Frame) -> Snapshot {
        Snapshot::from_parts(orr_bridge::SnapshotParts {
            seq: 1,
            tick: 0,
            verified_tick: 0,
            tick_rate: 60,
            predicted: Arc::new(frame),
            predicted_prev: None,
            verified: None,
            stats: Default::default(),
            last_rollback: None,
            timeline: None,
        })
    }

    #[test]
    fn fractional_time_accumulates_and_entity_reuse_resets_cursor() {
        let fixture = Fixture::new();
        fixture.install();
        let mut scene = SpriteScene::open(&fixture.0).unwrap();
        let mut registry = orr_ecs::ComponentRegistryBuilder::new();
        registry.register_component::<PlayerTag>("PlayerTag");
        let mut frame = orr_ecs::Frame::new(registry.build());
        let first = frame.spawn();
        frame.add(first, PlayerTag { slot: 0 });
        let first_snapshot = snapshot(frame.clone());
        let mut item = crate::arena_view::arena_floor();
        item.entity = first;
        for _ in 0..4 {
            scene.update(
                Duration::from_micros(400),
                Some(&first_snapshot),
                0,
                false,
                &mut vec![item],
            );
        }
        assert_eq!(scene.playback.elapsed_ms(), 1);
        assert_eq!(scene.elapsed_remainder, Duration::from_micros(600));
        assert!(frame.despawn(first));
        let next = frame.spawn();
        assert_eq!(next.index, first.index);
        assert_ne!(next.version, first.version);
        frame.add(next, PlayerTag { slot: 0 });
        item.entity = next;
        let next_snapshot = snapshot(frame);
        scene.update(
            Duration::ZERO,
            Some(&next_snapshot),
            0,
            false,
            &mut vec![item],
        );
        assert_eq!(scene.follow.target(), Some(&next));
        assert_eq!(scene.playback.elapsed_ms(), 0);
        assert_eq!(scene.elapsed_remainder, Duration::ZERO);
        assert!(!scene.moving);
        assert_eq!(scene.list.sprites[0].region, 10);
    }
}
