//! Runtime compositor for already-admitted authored sprite atlases.
//!
//! `sprites` is supplied in authoritative global order (the project runtime
//! currently orders it by entity GUID). Atlas switches are rendered as
//! sequential contiguous runs so alpha compositing never regroups the whole
//! frame by atlas. `SpriteInstance::order` is intentionally normalized inside
//! each run: callers cannot use a local atlas order to override global order.

use std::{collections::BTreeMap, sync::Arc};

use crate::project_sprites::{Asset, AssetKey};
use orr_render::{
    Camera, RenderList, Renderer, SpriteDrawList, SpriteInstance, SpriteRenderer,
    orr_rhi::{Acquire, Rhi, TextureFormat, Wgpu, WgpuOptions},
};
use winit::window::Window;

/// Maximum number of authored instances admitted to one composed frame.
/// This also keeps the run-local stable draw indices comfortably in `i32`.
pub const MAX_PROJECT_SPRITES: usize = 4096;

/// One sprite paired with the installed project asset that owns its region.
#[derive(Clone, Debug)]
pub struct SpriteDraw {
    pub asset: AssetKey,
    pub instance: SpriteInstance,
}

/// Draws Arena shapes followed by ordered project-authored sprites.
///
/// Each project asset has one uploaded atlas renderer. A draw keeps the input
/// slice order across atlas boundaries by issuing one pass for each contiguous
/// asset run; it never batches non-adjacent runs together.
pub struct ProjectCompositor<R: Rhi> {
    shapes: Renderer<R>,
    sprites: BTreeMap<AssetKey, SpriteRenderer<R>>,
}

impl<R: Rhi> ProjectCompositor<R> {
    /// Upload all assets that were admitted by the project runtime before GPU
    /// construction. Runtime file access and asset policy stay outside here.
    pub fn new(
        rhi: R,
        format: TextureFormat,
        assets: &BTreeMap<AssetKey, Asset>,
    ) -> Result<Self, String> {
        let mut shapes = Renderer::new(rhi.clone(), format);
        let mut sprites = BTreeMap::new();
        for (key, asset) in assets {
            let mut renderer =
                SpriteRenderer::new(rhi.clone(), format, asset.document.clone(), &asset.rgba)
                    .map_err(|error| format!("project sprite asset {key:?}: {error}"))?;
            renderer.clear = None;
            sprites.insert(key.clone(), renderer);
        }
        // Shapes own the first pass and its background clear. Sprite runs then
        // blend over those pixels without clearing them.
        shapes.clear = Some(orr_render::DEFAULT_CLEAR);
        Ok(Self { shapes, sprites })
    }

    /// Draw shapes and then sprites in the exact order of `sprites`.
    pub fn draw(
        &mut self,
        view: &R::TextureView,
        size: (u32, u32),
        shapes: &RenderList,
        sprites: &[SpriteDraw],
        camera: &Camera,
    ) -> Result<(), String> {
        if sprites.len() > MAX_PROJECT_SPRITES {
            return Err(format!(
                "project sprite count {} exceeds {MAX_PROJECT_SPRITES}",
                sprites.len()
            ));
        }
        validate_camera(camera)?;

        // Check every asset, region, and instance before the first submission.
        // A bad later item must not leave a partially rendered frame behind.
        for (index, sprite) in sprites.iter().enumerate() {
            let renderer = self.sprites.get(&sprite.asset).ok_or_else(|| {
                format!(
                    "project sprite {index} references unknown asset {:?}",
                    sprite.asset
                )
            })?;
            validate_instance(index, sprite.instance, renderer.document())?;
        }

        self.shapes.draw(view, size, shapes, camera);
        let mut start = 0;
        while start < sprites.len() {
            let asset = &sprites[start].asset;
            let mut end = start + 1;
            while end < sprites.len() && sprites[end].asset == *asset {
                end += 1;
            }

            let mut list = SpriteDrawList::default();
            for (run_index, draw) in sprites[start..end].iter().enumerate() {
                let mut instance = draw.instance;
                // Authored project bindings do not define transform flips.
                instance.flip_x = false;
                instance.flip_y = false;
                // `SpriteRenderer` stable-sorts by this field. Reindex each
                // contiguous run to preserve the caller's already-global order.
                instance.order = run_index as i32;
                list.push(instance);
            }
            let renderer = self
                .sprites
                .get_mut(asset)
                .expect("all project assets were validated before drawing");
            renderer
                .draw(view, size, &list, camera)
                .map_err(|error| format!("project sprite run {start}..{end}: {error}"))?;
            start = end;
        }
        Ok(())
    }
}

fn validate_camera(camera: &Camera) -> Result<(), String> {
    if !camera.center.iter().all(|value| value.is_finite())
        || !camera.half_extent.is_finite()
        || camera.half_extent < Camera::MIN_HALF_EXTENT
    {
        return Err(
            "sprite camera must be finite with extent at least Camera::MIN_HALF_EXTENT".into(),
        );
    }
    Ok(())
}

fn validate_instance(
    index: usize,
    instance: SpriteInstance,
    document: &orr_sprite::SpriteDocument,
) -> Result<(), String> {
    if !instance
        .position
        .iter()
        .chain(instance.size.iter())
        .chain(instance.tint.iter())
        .all(|value| value.is_finite())
        || !instance.rotation.is_finite()
        || instance.size.iter().any(|value| *value <= 0.0)
        || instance
            .tint
            .iter()
            .any(|value| !(0.0..=1.0).contains(value))
    {
        return Err(format!("invalid project sprite instance {index}"));
    }
    if document.uv_rect(instance.region).is_none() {
        return Err(format!(
            "project sprite instance {index} references unknown region {}",
            instance.region
        ));
    }
    Ok(())
}

/// Winit surface adapter using the same compositor as headless readback tests.
pub struct ProjectWindow {
    rhi: Wgpu,
    surface: <Wgpu as Rhi>::Surface,
    size: (u32, u32),
    format: TextureFormat,
    compositor: ProjectCompositor<Wgpu>,
}

impl ProjectWindow {
    pub fn new(
        window: Arc<Window>,
        vsync: bool,
        assets: &BTreeMap<AssetKey, Asset>,
    ) -> Result<Self, String> {
        let inner = window.inner_size();
        let size = (inner.width.max(1), inner.height.max(1));
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
        let compositor = ProjectCompositor::new(rhi.clone(), format, assets)?;
        Ok(Self {
            rhi,
            surface,
            size,
            format,
            compositor,
        })
    }

    pub fn rhi(&self) -> &Wgpu {
        &self.rhi
    }

    pub fn format(&self) -> TextureFormat {
        self.format
    }

    pub fn adapter_name(&self) -> String {
        self.rhi.adapter_name()
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.size = (width, height);
        self.rhi.resize_surface(&mut self.surface, width, height);
    }

    pub fn render(
        &mut self,
        shapes: &RenderList,
        sprites: &[SpriteDraw],
        camera: &Camera,
    ) -> Result<(), String> {
        let Acquire::Frame(frame) = self.rhi.acquire_frame(&mut self.surface) else {
            return Ok(());
        };
        let view = self.rhi.frame_view(&frame);
        self.compositor
            .draw(view, self.size, shapes, sprites, camera)?;
        self.rhi.present(frame);
        Ok(())
    }
}
