//! Optional nearest-filtered atlas renderer. Asset bytes are straight-alpha
//! sRGB RGBA8; sampling decodes RGB before applying the linear tint and blending.
//! No image decoder, simulation state or clock is required.
use crate::renderer::Growing;
use crate::{Camera, SpriteDrawList};
use bytemuck::{Pod, Zeroable};
use orr_rhi::{
    Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, PipelineDesc, Rhi,
    SamplerDesc, TextureDesc, TextureFormat, TextureUpload, TextureUploadError, TextureUsage,
    VertexAttr, VertexFormat, VertexLayout, VertexStep,
};
use orr_sprite::SpriteDocument;

/// Conservative atlas limit supported by the engine's native and WebGL2
/// devices. Larger assets must be split into atlases before renderer creation.
pub const MAX_SPRITE_ATLAS_EXTENT: u32 = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpriteRenderError {
    Upload(TextureUploadError),
    AtlasTooLarge { width: u32, height: u32 },
    MissingRegion(u32),
    InvalidInstance(usize),
    InvalidCamera,
    TooManyInstances,
}
impl std::fmt::Display for SpriteRenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Upload(e) => write!(f, "sprite atlas: {e}"),
            Self::AtlasTooLarge { width, height } => write!(f,
                "sprite atlas {width}x{height} exceeds {MAX_SPRITE_ATLAS_EXTENT} pixels per dimension"),
            Self::MissingRegion(id) => write!(f, "unknown sprite region {id}"),
            Self::InvalidInstance(i) => write!(f, "invalid sprite instance {i}"),
            Self::InvalidCamera => f.write_str("sprite camera must be finite with extent at least Camera::MIN_HALF_EXTENT"),
            Self::TooManyInstances => f.write_str("sprite count exceeds u32 draw range"),
        }
    }
}
impl std::error::Error for SpriteRenderError {}
impl From<TextureUploadError> for SpriteRenderError {
    fn from(e: TextureUploadError) -> Self {
        Self::Upload(e)
    }
}
// Validate dimensions before allocation as byte length alone cannot constrain
// a skinny atlas: 1048576x1 only occupies 4 MiB but exceeds GPU dimensions.
fn atlas_stride(width: u32, height: u32, byte_len: usize) -> Result<u32, SpriteRenderError> {
    if width == 0 || height == 0 {
        return Err(TextureUploadError::EmptyExtent.into());
    }
    if width > MAX_SPRITE_ATLAS_EXTENT || height > MAX_SPRITE_ATLAS_EXTENT {
        return Err(SpriteRenderError::AtlasTooLarge { width, height });
    }
    let stride = width
        .checked_mul(4)
        .ok_or(TextureUploadError::SizeOverflow)?;
    let expected = u64::from(stride) * u64::from(height);
    if u64::try_from(byte_len).ok() != Some(expected) {
        return Err(TextureUploadError::InvalidDataLength {
            expected,
            actual: byte_len,
        }
        .into());
    }
    Ok(stride)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    center: [f32; 2],
    scale: [f32; 2],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    center: [f32; 2],
    size: [f32; 2],
    rotation: f32,
    uv: [f32; 4],
    tint: [f32; 4],
}
const ATTRS: [VertexAttr; 5] = [
    VertexAttr {
        location: 0,
        format: VertexFormat::Float32x2,
        offset: 0,
    },
    VertexAttr {
        location: 1,
        format: VertexFormat::Float32x2,
        offset: 8,
    },
    VertexAttr {
        location: 2,
        format: VertexFormat::Float32,
        offset: 16,
    },
    VertexAttr {
        location: 3,
        format: VertexFormat::Float32x4,
        offset: 20,
    },
    VertexAttr {
        location: 4,
        format: VertexFormat::Float32x4,
        offset: 36,
    },
];

/// One immutable document and atlas per renderer; recreate explicitly when an
/// asset changes. Drawing validates the entire list before any GPU writes.
/// Uses one single-sample pass and one instanced draw, with stable layer order.
/// Set `clear = None` to overlay existing geometry on the same target.
pub struct SpriteRenderer<B: Rhi> {
    rhi: B,
    format: TextureFormat,
    document: SpriteDocument,
    pipeline: B::Pipeline,
    globals: B::Buffer,
    bind: B::BindGroup,
    instances: Growing<B>,
    prepared: Vec<Instance>,
    pub clear: Option<[f64; 4]>,
}
impl<B: Rhi> SpriteRenderer<B> {
    /// Upload tightly packed top-to-bottom sRGB RGBA8 with straight alpha.
    pub fn new(
        rhi: B,
        format: TextureFormat,
        document: SpriteDocument,
        rgba8: &[u8],
    ) -> Result<Self, SpriteRenderError> {
        let a = document.atlas();
        let stride = atlas_stride(a.width, a.height, rgba8.len())?;
        let texture = rhi.create_texture(&TextureDesc {
            label: "sprite atlas",
            width: a.width,
            height: a.height,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsage::TEXTURE_BINDING | TextureUsage::COPY_DST,
            sample_count: 1,
            view_formats: &[],
        });
        rhi.write_texture_rgba8(
            &texture,
            &TextureUpload {
                origin: [0, 0],
                width: a.width,
                height: a.height,
                bytes_per_row: stride,
                data: rgba8,
            },
        )?;
        let view = rhi.create_texture_view(&texture, None);
        let sampler = rhi.create_sampler(&SamplerDesc {
            linear: false,
            compare: false,
        });
        let globals = rhi.create_buffer(&BufferDesc {
            label: "sprite camera",
            size: 16,
            usage: BufferUsage::UNIFORM | BufferUsage::COPY_DST,
        });
        let shader = rhi.create_shader("sprites", include_str!("shader_sprite.wgsl"));
        let pipeline = rhi.create_pipeline(&PipelineDesc::color(
            "sprites",
            &shader,
            ("vs_sprite", "fs_sprite"),
            &[VertexLayout {
                stride: std::mem::size_of::<Instance>() as u64,
                step: VertexStep::Instance,
                attrs: &ATTRS,
            }],
            format,
            Blend::Alpha,
        ));
        let bind = rhi.create_bind_group(
            &pipeline,
            0,
            &[
                Binding::Uniform {
                    binding: 0,
                    buffer: &globals,
                },
                Binding::Texture {
                    binding: 1,
                    view: &view,
                },
                Binding::Sampler {
                    binding: 2,
                    sampler: &sampler,
                },
            ],
        );
        let instances = Growing::new(&rhi, "sprite instances", std::mem::size_of::<Instance>());
        Ok(Self {
            rhi,
            format,
            document,
            pipeline,
            globals,
            bind,
            instances,
            prepared: Vec::new(),
            clear: Some(crate::DEFAULT_CLEAR),
        })
    }
    pub fn document(&self) -> &SpriteDocument {
        &self.document
    }
    pub fn format(&self) -> TextureFormat {
        self.format
    }
    pub fn rhi(&self) -> &B {
        &self.rhi
    }

    pub fn draw(
        &mut self,
        view: &B::TextureView,
        size: (u32, u32),
        list: &SpriteDrawList,
        camera: &Camera,
    ) -> Result<(), SpriteRenderError> {
        if !camera.center.iter().all(|v| v.is_finite())
            || !camera.half_extent.is_finite()
            || camera.half_extent < Camera::MIN_HALF_EXTENT
        {
            return Err(SpriteRenderError::InvalidCamera);
        }
        let count =
            u32::try_from(list.sprites.len()).map_err(|_| SpriteRenderError::TooManyInstances)?;
        self.prepared.clear();
        // Stable sort is deliberately independent of region: alpha compositing
        // must not be reordered merely to batch atlas regions.
        let mut ordered: Vec<_> = list.sprites.iter().enumerate().collect();
        ordered.sort_by_key(|(_, s)| s.order);
        for (i, s) in ordered {
            if !s
                .position
                .iter()
                .chain(s.size.iter())
                .chain(s.tint.iter())
                .all(|v| v.is_finite())
                || !s.rotation.is_finite()
                || s.size.iter().any(|v| *v <= 0.0)
                || s.tint.iter().any(|v| !(0.0..=1.0).contains(v))
            {
                return Err(SpriteRenderError::InvalidInstance(i));
            }
            let mut uv = self
                .document
                .uv_rect(s.region)
                .ok_or(SpriteRenderError::MissingRegion(s.region))?;
            if s.flip_x {
                uv.swap(0, 2);
            }
            if s.flip_y {
                uv.swap(1, 3);
            }
            self.prepared.push(Instance {
                center: s.position,
                size: s.size,
                rotation: s.rotation,
                uv,
                tint: s.tint,
            });
        }
        self.rhi.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&Globals {
                center: camera.center,
                scale: camera.scale(size.0, size.1),
            }),
        );
        let mut commands = Vec::new();
        if count != 0 {
            self.instances.reserve(&self.rhi, self.prepared.len());
            self.rhi.write_buffer(
                &self.instances.buffer,
                0,
                bytemuck::cast_slice(&self.prepared),
            );
            commands.extend([
                Command::SetPipeline(&self.pipeline),
                Command::SetBindGroup(0, &self.bind),
                Command::SetVertexBuffer(0, &self.instances.buffer),
                Command::Draw {
                    vertices: 0..6,
                    instances: 0..count,
                },
            ]);
        }
        let mut encoder = self.rhi.create_encoder("sprite frame");
        self.rhi.encode_render_pass(
            &mut encoder,
            "sprites",
            &ColorAttachment {
                view,
                clear: self.clear,
                resolve: None,
            },
            &commands,
        );
        self.rhi.submit(encoder);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_dimensions_are_bounded_before_resource_creation() {
        for (width, height, bytes) in [
            (1_048_576, 1, 4_194_304),
            (1, 1_048_576, 4_194_304),
            (2049, 1, 8196),
            (1, 2049, 8196),
            (u32::MAX, u32::MAX, 0),
        ] {
            assert_eq!(
                atlas_stride(width, height, bytes),
                Err(SpriteRenderError::AtlasTooLarge { width, height })
            );
        }
        assert_eq!(atlas_stride(2048, 2048, 2048 * 2048 * 4), Ok(8192));
        assert_eq!(
            atlas_stride(0, 1, 0),
            Err(SpriteRenderError::Upload(TextureUploadError::EmptyExtent))
        );
        assert!(matches!(
            atlas_stride(2, 2, 15),
            Err(SpriteRenderError::Upload(
                TextureUploadError::InvalidDataLength { .. }
            ))
        ));
    }
}
