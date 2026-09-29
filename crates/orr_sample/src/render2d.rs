//! A minimal 2D renderer on wgpu: one instanced draw of colored quads and
//! circles, alpha blended, with a fixed camera that fits the arena.
//! The shader is WGSL (design doc 4.3). This stands in for the future
//! `orr_rhi` / `orr_render` crates.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use orr_view::{RenderItem, Shape};
use winit::window::Window;

/// One instance as the shader reads it (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    center: [f32; 2],
    half_size: f32,
    shape: u32,
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    center: [f32; 2],
    scale: [f32; 2],
}

/// Camera: looks at `center` and shows `half_extent` world units from the
/// center to the nearer window edge.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub center: [f32; 2],
    pub half_extent: f32,
}

impl Camera {
    /// World to clip scale for a window of `width` x `height` pixels.
    pub fn scale(&self, width: u32, height: u32) -> [f32; 2] {
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        let per_unit = 1.0 / self.half_extent;
        if w >= h {
            [per_unit * h / w, per_unit]
        } else {
            [per_unit, per_unit * w / h]
        }
    }
}

const INITIAL_INSTANCES: usize = 1024;

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    scratch: Vec<Instance>,
    adapter_name: String,
}

impl Renderer {
    /// Creates the GPU state for `window`. `vsync` off asks for a present
    /// mode without waiting for the display (to measure raw speed).
    pub fn new(window: Arc<Window>, vsync: bool) -> Result<Self, String> {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window).map_err(|e| format!("create_surface: {e}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| format!("request_adapter: {e}"))?;
        let adapter_name = adapter.get_info().name;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(|e| format!("request_device: {e}"))?;

        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("surface not supported by the adapter")?;
        let modes = surface.get_capabilities(&adapter).present_modes;
        config.present_mode = if vsync {
            wgpu::PresentMode::Fifo
        } else if modes.contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("orr_sample 2d"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals group"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals_buffer.as_entire_binding() }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("2d layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("2d pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32, 2 => Uint32, 3 => Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let instance_buffer = Self::make_instance_buffer(&device, INITIAL_INSTANCES);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            globals_buffer,
            bind_group,
            instance_buffer,
            instance_capacity: INITIAL_INSTANCES,
            scratch: Vec::new(),
            adapter_name,
        })
    }

    fn make_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: (capacity * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Draws `items` in order (later on top). Returns `false` if the frame
    /// was skipped (window hidden, surface being reconfigured).
    pub fn render(&mut self, items: &[RenderItem], camera: &Camera) -> bool {
        use wgpu::CurrentSurfaceTexture as Cst;
        let frame = match self.surface.get_current_texture() {
            Cst::Success(frame) => frame,
            Cst::Suboptimal(frame) => {
                self.surface.configure(&self.device, &self.config);
                frame
            }
            Cst::Outdated | Cst::Lost => {
                self.surface.configure(&self.device, &self.config);
                return false;
            }
            Cst::Timeout | Cst::Occluded | Cst::Validation => return false,
        };

        self.scratch.clear();
        self.scratch.extend(items.iter().map(|i| Instance {
            center: [i.transform.pos.x, i.transform.pos.y],
            half_size: i.style.size,
            shape: match i.style.shape {
                Shape::Circle => 0,
                Shape::Quad => 1,
            },
            color: i.style.color,
        }));
        if self.scratch.len() > self.instance_capacity {
            self.instance_capacity = self.scratch.len().next_power_of_two();
            self.instance_buffer = Self::make_instance_buffer(&self.device, self.instance_capacity);
        }
        self.queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&self.scratch));
        let globals =
            Globals { center: camera.center, scale: camera.scale(self.config.width, self.config.height) };
        self.queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));

        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("2d"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.02, g: 0.02, b: 0.035, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
            pass.draw(0..6, 0..self.scratch.len() as u32);
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        true
    }
}
