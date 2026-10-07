//! The wgpu backend of [`Rhi`].

use std::sync::Arc;

use crate::{
    Acquire, Binding, Blend, BufferDesc, BufferUsage, ColorAttachment, Command, Compare, Cull, DepthAttachment,
    PipelineDesc, Rhi, SamplerDesc, TextureDesc, TextureFormat, TextureUsage, Topology, VertexFormat, VertexStep,
    TextureUpload, TextureUploadError, WindowHandle,
};

/// How to pick the adapter.
#[derive(Clone, Copy, Debug)]
pub struct WgpuOptions {
    pub high_performance: bool,
    /// When no hardware adapter exists, try a software one (WARP on Windows,
    /// llvmpipe/lavapipe on Linux).
    pub allow_software_fallback: bool,
    /// Skip hardware adapters and use only the software one (for tests).
    pub force_software: bool,
}

impl Default for WgpuOptions {
    fn default() -> Self {
        Self { high_performance: true, allow_software_fallback: true, force_software: false }
    }
}

/// Handle of a wgpu device. Cloning shares the device and queue.
#[derive(Clone)]
pub struct Wgpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

/// A window surface with its current configuration.
pub struct WgpuSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    format: TextureFormat,
    /// The surface textures can be copied from (screenshots).
    can_copy: bool,
}

/// One acquired surface frame.
pub struct WgpuFrame {
    texture: wgpu::SurfaceTexture,
    view: wgpu::TextureView,
    can_copy: bool,
}

fn to_wgpu_format(f: TextureFormat) -> wgpu::TextureFormat {
    match f {
        TextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
        TextureFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        TextureFormat::Bgra8Unorm => wgpu::TextureFormat::Bgra8Unorm,
        TextureFormat::Bgra8UnormSrgb => wgpu::TextureFormat::Bgra8UnormSrgb,
        TextureFormat::Depth32Float => wgpu::TextureFormat::Depth32Float,
    }
}

fn from_wgpu_format(f: wgpu::TextureFormat) -> Option<TextureFormat> {
    Some(match f {
        wgpu::TextureFormat::Rgba8Unorm => TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb => TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8Unorm => TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Bgra8UnormSrgb => TextureFormat::Bgra8UnormSrgb,
        _ => return None,
    })
}

fn to_wgpu_vertex(f: VertexFormat) -> wgpu::VertexFormat {
    match f {
        VertexFormat::Float32 => wgpu::VertexFormat::Float32,
        VertexFormat::Float32x2 => wgpu::VertexFormat::Float32x2,
        VertexFormat::Float32x3 => wgpu::VertexFormat::Float32x3,
        VertexFormat::Float32x4 => wgpu::VertexFormat::Float32x4,
        VertexFormat::Uint32 => wgpu::VertexFormat::Uint32,
    }
}

fn buffer_usages(u: BufferUsage) -> wgpu::BufferUsages {
    let mut r = wgpu::BufferUsages::empty();
    for (flag, w) in [
        (BufferUsage::VERTEX, wgpu::BufferUsages::VERTEX),
        (BufferUsage::UNIFORM, wgpu::BufferUsages::UNIFORM),
        (BufferUsage::COPY_SRC, wgpu::BufferUsages::COPY_SRC),
        (BufferUsage::COPY_DST, wgpu::BufferUsages::COPY_DST),
        (BufferUsage::INDEX, wgpu::BufferUsages::INDEX),
    ] {
        if u.contains(flag) {
            r |= w;
        }
    }
    r
}

fn texture_usages(u: TextureUsage) -> wgpu::TextureUsages {
    let mut r = wgpu::TextureUsages::empty();
    for (flag, w) in [
        (TextureUsage::RENDER_ATTACHMENT, wgpu::TextureUsages::RENDER_ATTACHMENT),
        (TextureUsage::TEXTURE_BINDING, wgpu::TextureUsages::TEXTURE_BINDING),
        (TextureUsage::COPY_SRC, wgpu::TextureUsages::COPY_SRC),
        (TextureUsage::COPY_DST, wgpu::TextureUsages::COPY_DST),
    ] {
        if u.contains(flag) {
            r |= w;
        }
    }
    r
}

impl Wgpu {
    /// A device without a window (offscreen rendering, tests). `Err` when no
    /// adapter is available.
    pub fn headless(opts: WgpuOptions) -> Result<Self, String> {
        let instance = wgpu::Instance::default();
        Self::from_instance(instance, None, opts)
    }

    /// A device for `window` plus its surface. `size` is the window's
    /// inner size in pixels.
    pub fn for_window<W: WindowHandle>(
        window: Arc<W>,
        size: (u32, u32),
        vsync: bool,
        opts: WgpuOptions,
    ) -> Result<(Self, WgpuSurface), String> {
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window).map_err(|e| format!("create_surface: {e}"))?;
        let wgpu = Self::from_instance(instance, Some(&surface), opts)?;
        let surface = wgpu.configure_new_surface(surface, size, vsync)?;
        Ok((wgpu, surface))
    }

    /// A surface for another window on the same device.
    pub fn create_surface<W: WindowHandle>(
        &self,
        window: Arc<W>,
        size: (u32, u32),
        vsync: bool,
    ) -> Result<WgpuSurface, String> {
        let surface = self.instance.create_surface(window).map_err(|e| format!("create_surface: {e}"))?;
        self.configure_new_surface(surface, size, vsync)
    }

    fn from_instance(
        instance: wgpu::Instance,
        compatible: Option<&wgpu::Surface<'static>>,
        opts: WgpuOptions,
    ) -> Result<Self, String> {
        pollster::block_on(Self::from_instance_async(instance, compatible, opts))
    }

    async fn from_instance_async(
        instance: wgpu::Instance,
        compatible: Option<&wgpu::Surface<'static>>,
        opts: WgpuOptions,
    ) -> Result<Self, String> {
        let power = if opts.high_performance {
            wgpu::PowerPreference::HighPerformance
        } else {
            wgpu::PowerPreference::LowPower
        };
        let request = |fallback: bool| {
            instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: power,
                compatible_surface: compatible,
                force_fallback_adapter: fallback,
                ..Default::default()
            })
        };
        let adapter = if opts.force_software {
            request(true).await
        } else {
            match request(false).await {
                Ok(a) => Ok(a),
                Err(e) if opts.allow_software_fallback => request(true).await.map_err(|_| e),
                Err(e) => Err(e),
            }
        }
        .map_err(|e| format!("request_adapter: {e}"))?;
        #[allow(unused_mut)]
        let mut desc = wgpu::DeviceDescriptor::default();
        // WebGL2 has no compute shaders, storage buffers or large limits: ask for what it can do.
        #[cfg(target_arch = "wasm32")]
        if adapter.get_info().backend == wgpu::Backend::Gl {
            desc.required_limits = wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());
        }
        let (device, queue) = adapter.request_device(&desc).await.map_err(|e| format!("request_device: {e}"))?;
        Ok(Self { instance, adapter, device, queue })
    }

    fn configure_new_surface(
        &self,
        surface: wgpu::Surface<'static>,
        size: (u32, u32),
        vsync: bool,
    ) -> Result<WgpuSurface, String> {
        let mut config = surface
            .get_default_config(&self.adapter, size.0.max(1), size.1.max(1))
            .ok_or("surface not supported by the adapter")?;
        let caps = surface.get_capabilities(&self.adapter);
        let can_copy = caps.usages.contains(wgpu::TextureUsages::COPY_SRC);
        if can_copy {
            config.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        let modes = caps.present_modes;
        config.present_mode = if vsync {
            wgpu::PresentMode::Fifo
        } else if modes.contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        let format = from_wgpu_format(config.format).ok_or_else(|| format!("unsupported surface format {:?}", config.format))?;
        surface.configure(&self.device, &config);
        Ok(WgpuSurface { surface, config, format, can_copy })
    }

    /// Wraps a device that someone else created (for example eframe's
    /// `egui_wgpu::RenderState`), so the renderer can draw with the same
    /// device and queue that a UI toolkit uses. The parts must belong together.
    pub fn from_parts(instance: wgpu::Instance, adapter: wgpu::Adapter, device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self { instance, adapter, device, queue }
    }

    /// The raw wgpu device, for integrations such as egui-wgpu.
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    /// True when the adapter is a software rasterizer (WARP, llvmpipe).
    pub fn is_software(&self) -> bool {
        self.adapter.get_info().device_type == wgpu::DeviceType::Cpu
    }
}

/// Which browser graphics API [`Wgpu::for_canvas`] may use.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WebBackend {
    /// WebGPU when the browser has a working adapter, WebGL2 otherwise.
    #[default]
    Auto,
    /// WebGPU only.
    WebGpu,
    /// WebGL2 only.
    WebGl,
}

#[cfg(target_arch = "wasm32")]
impl Wgpu {
    /// A device and a surface for a canvas element (browser). The device is created asynchronously,
    /// so this cannot block the page; `size` is the canvas size in pixels.
    pub async fn for_canvas(
        canvas: wgpu::web_sys::HtmlCanvasElement,
        size: (u32, u32),
        backend: WebBackend,
    ) -> Result<(Self, WgpuSurface), String> {
        let backends = match backend {
            WebBackend::Auto => wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL,
            WebBackend::WebGpu => wgpu::Backends::BROWSER_WEBGPU,
            WebBackend::WebGl => wgpu::Backends::GL,
        };
        let desc = wgpu::InstanceDescriptor { backends, ..wgpu::InstanceDescriptor::new_without_display_handle() };
        let instance = if backend == WebBackend::Auto {
            wgpu::util::new_instance_with_webgpu_detection(desc).await
        } else {
            wgpu::Instance::new(desc)
        };
        let surface =
            instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas)).map_err(|e| format!("create_surface: {e}"))?;
        let opts = WgpuOptions { allow_software_fallback: false, ..WgpuOptions::default() };
        let wgpu = Self::from_instance_async(instance, Some(&surface), opts).await?;
        let surface = wgpu.configure_new_surface(surface, size, true)?;
        Ok((wgpu, surface))
    }

    /// The graphics API in use, as wgpu names the backend (`BrowserWebGpu`, `Gl`).
    pub fn backend_name(&self) -> String {
        format!("{:?}", self.adapter.get_info().backend)
    }
}

impl Rhi for Wgpu {
    type Buffer = wgpu::Buffer;
    type Texture = wgpu::Texture;
    type TextureView = wgpu::TextureView;
    type Shader = wgpu::ShaderModule;
    type Pipeline = wgpu::RenderPipeline;
    type BindGroup = wgpu::BindGroup;
    type Sampler = wgpu::Sampler;
    type Encoder = wgpu::CommandEncoder;
    type Surface = WgpuSurface;
    type Frame = WgpuFrame;

    fn adapter_name(&self) -> String {
        self.adapter.get_info().name
    }

    fn create_buffer(&self, desc: &BufferDesc) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(desc.label),
            size: desc.size,
            usage: buffer_usages(desc.usage),
            mapped_at_creation: false,
        })
    }

    fn write_buffer(&self, buffer: &wgpu::Buffer, offset: u64, data: &[u8]) {
        self.queue.write_buffer(buffer, offset, data);
    }

    fn texture_view_formats_supported(&self) -> bool {
        self.adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::VIEW_FORMATS)
    }

    fn create_texture(&self, desc: &TextureDesc) -> wgpu::Texture {
        let view_formats: Vec<wgpu::TextureFormat> = desc.view_formats.iter().map(|&f| to_wgpu_format(f)).collect();
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(desc.label),
            size: wgpu::Extent3d { width: desc.width.max(1), height: desc.height.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: desc.sample_count.max(1),
            dimension: wgpu::TextureDimension::D2,
            format: to_wgpu_format(desc.format),
            usage: texture_usages(desc.usage),
            view_formats: &view_formats,
        })
    }

    fn copy_texture(&self, source: &wgpu::Texture, destination: &wgpu::Texture) {
        assert_eq!(source.size(), destination.size(), "whole-texture copy requires equal extents");
        let mut encoder = self.create_encoder("offscreen sample copy");
        encoder.copy_texture_to_texture(
            source.as_image_copy(),
            destination.as_image_copy(),
            source.size(),
        );
        self.submit(encoder);
    }

    fn write_texture_rgba8(
        &self,
        texture: &wgpu::Texture,
        upload: &TextureUpload<'_>,
    ) -> Result<(), TextureUploadError> {
        if texture.dimension() != wgpu::TextureDimension::D2 || texture.depth_or_array_layers() != 1 {
            return Err(TextureUploadError::UnsupportedTexture);
        }
        let format = from_wgpu_format(texture.format()).ok_or(TextureUploadError::UnsupportedFormat)?;
        let usage = if texture.usage().contains(wgpu::TextureUsages::COPY_DST) {
            TextureUsage::COPY_DST
        } else {
            TextureUsage::default()
        };
        crate::validate_texture_upload(
            &TextureDesc {
                label: "upload validation",
                width: texture.width(),
                height: texture.height(),
                format,
                usage,
                sample_count: texture.sample_count(),
                view_formats: &[],
            },
            upload,
        )?;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: upload.origin[0], y: upload.origin[1], z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            upload.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(upload.bytes_per_row),
                rows_per_image: Some(upload.height),
            },
            wgpu::Extent3d { width: upload.width, height: upload.height, depth_or_array_layers: 1 },
        );
        Ok(())
    }

    fn create_texture_view(&self, texture: &wgpu::Texture, format: Option<TextureFormat>) -> wgpu::TextureView {
        texture.create_view(&wgpu::TextureViewDescriptor { format: format.map(to_wgpu_format), ..Default::default() })
    }

    fn create_sampler(&self, desc: &SamplerDesc) -> wgpu::Sampler {
        let filter = if desc.linear { wgpu::FilterMode::Linear } else { wgpu::FilterMode::Nearest };
        self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("orr sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: filter,
            min_filter: filter,
            compare: desc.compare.then_some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        })
    }

    fn sample_count_supported(&self, format: TextureFormat, samples: u32) -> bool {
        // Adapter-specific counts are only usable when the device enabled
        // TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES; otherwise only the
        // WebGPU-guaranteed counts (1 and 4) pass validation.
        let format = to_wgpu_format(format);
        let features = self.device.features();
        let flags = if features.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES) {
            self.adapter.get_texture_format_features(format).flags
        } else {
            format.guaranteed_format_features(features).flags
        };
        samples == 1 || flags.sample_count_supported(samples)
    }

    fn create_shader(&self, label: &str, wgsl: &str) -> wgpu::ShaderModule {
        self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        })
    }

    fn create_pipeline(&self, desc: &PipelineDesc<Self>) -> wgpu::RenderPipeline {
        // wgpu wants the attribute arrays to outlive the descriptor.
        let attrs: Vec<Vec<wgpu::VertexAttribute>> = desc
            .vertex_buffers
            .iter()
            .map(|b| {
                b.attrs
                    .iter()
                    .map(|a| wgpu::VertexAttribute {
                        format: to_wgpu_vertex(a.format),
                        offset: a.offset,
                        shader_location: a.location,
                    })
                    .collect()
            })
            .collect();
        let buffers: Vec<Option<wgpu::VertexBufferLayout>> = desc
            .vertex_buffers
            .iter()
            .zip(&attrs)
            .map(|(b, a)| {
                Some(wgpu::VertexBufferLayout {
                    array_stride: b.stride,
                    step_mode: match b.step {
                        VertexStep::Vertex => wgpu::VertexStepMode::Vertex,
                        VertexStep::Instance => wgpu::VertexStepMode::Instance,
                    },
                    attributes: a,
                })
            })
            .collect();
        let blend = match desc.blend {
            Blend::Opaque => None,
            Blend::Alpha => Some(wgpu::BlendState::ALPHA_BLENDING),
        };
        let targets: Vec<Option<wgpu::ColorTargetState>> = desc
            .color_format
            .map(|f| wgpu::ColorTargetState { format: to_wgpu_format(f), blend, write_mask: wgpu::ColorWrites::ALL })
            .into_iter()
            .map(Some)
            .collect();
        let depth_stencil = desc.depth.map(|d| wgpu::DepthStencilState {
            format: to_wgpu_format(d.format),
            depth_write_enabled: Some(d.write),
            depth_compare: Some(match d.compare {
                Compare::Less => wgpu::CompareFunction::Less,
                Compare::LessEqual => wgpu::CompareFunction::LessEqual,
                Compare::Always => wgpu::CompareFunction::Always,
            }),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState { constant: d.bias, slope_scale: d.slope_bias, clamp: 0.0 },
        });
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(desc.label),
            layout: None,
            vertex: wgpu::VertexState {
                module: desc.shader,
                entry_point: Some(desc.vs_entry),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            fragment: desc.color_format.map(|_| wgpu::FragmentState {
                module: desc.shader,
                entry_point: Some(desc.fs_entry),
                compilation_options: Default::default(),
                targets: &targets,
            }),
            primitive: wgpu::PrimitiveState {
                topology: match desc.topology {
                    Topology::TriangleList => wgpu::PrimitiveTopology::TriangleList,
                    Topology::LineList => wgpu::PrimitiveTopology::LineList,
                },
                cull_mode: match desc.cull {
                    Cull::None => None,
                    Cull::Back => Some(wgpu::Face::Back),
                    Cull::Front => Some(wgpu::Face::Front),
                },
                ..Default::default()
            },
            depth_stencil,
            multisample: wgpu::MultisampleState { count: desc.samples.max(1), mask: !0, alpha_to_coverage_enabled: false },
            multiview_mask: None,
            cache: None,
        })
    }

    fn create_bind_group(&self, pipeline: &wgpu::RenderPipeline, group: u32, bindings: &[Binding<Self>]) -> wgpu::BindGroup {
        let layout = pipeline.get_bind_group_layout(group);
        let entries: Vec<wgpu::BindGroupEntry> = bindings
            .iter()
            .map(|b| match b {
                Binding::Uniform { binding, buffer } => {
                    wgpu::BindGroupEntry { binding: *binding, resource: buffer.as_entire_binding() }
                }
                Binding::Texture { binding, view } => {
                    wgpu::BindGroupEntry { binding: *binding, resource: wgpu::BindingResource::TextureView(view) }
                }
                Binding::Sampler { binding, sampler } => {
                    wgpu::BindGroupEntry { binding: *binding, resource: wgpu::BindingResource::Sampler(sampler) }
                }
            })
            .collect();
        self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &entries })
    }

    fn create_encoder(&self, label: &str) -> wgpu::CommandEncoder {
        self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) })
    }

    fn encode_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        label: &str,
        color: Option<&ColorAttachment<Self>>,
        depth: Option<&DepthAttachment<Self>>,
        commands: &[Command<Self>],
    ) {
        let color_attachment = color.map(|c| {
            let load = match c.clear {
                Some([r, g, b, a]) => wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                None => wgpu::LoadOp::Load,
            };
            // With a resolve target the multisampled contents are not needed afterwards.
            let store = if c.resolve.is_some() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store };
            wgpu::RenderPassColorAttachment {
                view: c.view,
                depth_slice: None,
                resolve_target: c.resolve,
                ops: wgpu::Operations { load, store },
            }
        });
        let color_attachments: Vec<Option<wgpu::RenderPassColorAttachment>> = color_attachment.into_iter().map(Some).collect();
        let depth_attachment = depth.map(|d| wgpu::RenderPassDepthStencilAttachment {
            view: d.view,
            depth_ops: Some(wgpu::Operations {
                load: match d.clear {
                    Some(v) => wgpu::LoadOp::Clear(v),
                    None => wgpu::LoadOp::Load,
                },
                store: if d.store { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard },
            }),
            stencil_ops: None,
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: color_attachments.as_slice(),
            depth_stencil_attachment: depth_attachment,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        for c in commands {
            match c {
                Command::SetPipeline(p) => pass.set_pipeline(p),
                Command::SetBindGroup(i, g) => pass.set_bind_group(*i, *g, &[]),
                Command::SetVertexBuffer(i, b) => pass.set_vertex_buffer(*i, b.slice(..)),
                Command::SetIndexBuffer(b) => pass.set_index_buffer(b.slice(..), wgpu::IndexFormat::Uint32),
                Command::Draw { vertices, instances } => pass.draw(vertices.clone(), instances.clone()),
                Command::DrawIndexed { indices, base_vertex, instances } => {
                    pass.draw_indexed(indices.clone(), *base_vertex, instances.clone())
                }
            }
        }
    }

    fn submit(&self, encoder: wgpu::CommandEncoder) {
        self.queue.submit(Some(encoder.finish()));
    }

    fn wait_idle(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    fn read_texture(&self, texture: &wgpu::Texture) -> Vec<u8> {
        let (w, h) = (texture.width(), texture.height());
        let unpadded = w * 4;
        let padded = unpadded.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.create_encoder("readback");
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.submit(encoder);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().expect("map callback").expect("map readback buffer");
        let data = buffer.slice(..).get_mapped_range().expect("mapped range");
        let mut out = Vec::with_capacity((unpadded * h) as usize);
        for row in data.chunks(padded as usize).take(h as usize) {
            out.extend_from_slice(&row[..unpadded as usize]);
        }
        drop(data);
        buffer.unmap();
        out
    }

    fn surface_format(&self, surface: &WgpuSurface) -> TextureFormat {
        surface.format
    }

    fn resize_surface(&self, surface: &mut WgpuSurface, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        surface.config.width = width;
        surface.config.height = height;
        surface.surface.configure(&self.device, &surface.config);
    }

    fn acquire_frame(&self, surface: &mut WgpuSurface) -> Acquire<WgpuFrame> {
        use wgpu::CurrentSurfaceTexture as Cst;
        let texture = match surface.surface.get_current_texture() {
            Cst::Success(t) => t,
            Cst::Suboptimal(t) => {
                surface.surface.configure(&self.device, &surface.config);
                t
            }
            Cst::Outdated | Cst::Lost => {
                surface.surface.configure(&self.device, &surface.config);
                return Acquire::Skip;
            }
            Cst::Timeout | Cst::Occluded | Cst::Validation => return Acquire::Skip,
        };
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        Acquire::Frame(WgpuFrame { texture, view, can_copy: surface.can_copy })
    }

    fn frame_view<'a>(&self, frame: &'a WgpuFrame) -> &'a wgpu::TextureView {
        &frame.view
    }

    fn read_frame(&self, frame: &WgpuFrame) -> Option<Vec<u8>> {
        frame.can_copy.then(|| self.read_texture(&frame.texture.texture))
    }

    fn present(&self, frame: WgpuFrame) {
        self.queue.present(frame.texture);
    }
}
