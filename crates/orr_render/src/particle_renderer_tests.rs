use super::*;
use std::{cell::RefCell, rc::Rc};
use orr_rhi::{Acquire, SamplerDesc, TextureDesc, TextureFormatCapabilities};
#[derive(Default)]
struct Effects { buffers: Vec<u64>, pipelines: Vec<(Blend, DepthState)>, writes: usize, passes: usize, commands: usize, submits: usize }
#[derive(Clone)]
struct Mock { effects: Rc<RefCell<Effects>>, color: TextureFormatCapabilities, depth: TextureFormatCapabilities, samples: bool }
impl Default for Mock {
    fn default() -> Self {
        let caps = TextureFormatCapabilities { usages: TextureUsage::RENDER_ATTACHMENT, blendable: true, max_dimension_2d: 8192, ..TextureFormatCapabilities::default() };
        Self { effects: Rc::new(RefCell::new(Effects::default())), color: caps, depth: caps, samples: true }
    }
}
impl Rhi for Mock {
    type Buffer = u32; type Texture = u32; type TextureView = u32; type Shader = (); type Pipeline = u32; type BindGroup = u32; type Sampler = (); type Encoder = (); type Surface = (); type Frame = u32;
    fn adapter_name(&self) -> String { "CPU mock".into() }
    fn create_buffer(&self, desc: &BufferDesc) -> u32 { let mut e = self.effects.borrow_mut(); e.buffers.push(desc.size); e.buffers.len() as u32 }
    fn write_buffer(&self, _: &u32, _: u64, _: &[u8]) { self.effects.borrow_mut().writes += 1; }
    fn texture_view_formats_supported(&self) -> bool { false }
    fn create_texture(&self, _: &TextureDesc) -> u32 { panic!("particle renderer owns no targets") }
    fn copy_texture(&self, _: &u32, _: &u32) { panic!("no texture copies") }
    fn create_texture_view(&self, _: &u32, _: Option<TextureFormat>) -> u32 { panic!("no views") }
    fn create_sampler(&self, _: &SamplerDesc) {}
    fn texture_format_capabilities(&self, format: TextureFormat) -> TextureFormatCapabilities { if format.is_depth() { self.depth } else { self.color } }
    fn sample_count_supported(&self, _: TextureFormat, samples: u32) -> bool { self.samples && samples == 1 }
    fn create_shader(&self, _: &str, text: &str) { assert_eq!(text, SHADER); }
    fn create_pipeline(&self, desc: &PipelineDesc<Self>) -> u32 {
        assert_eq!(desc.samples, 1); assert_eq!(desc.vertex_buffers[0].stride, 32);
        let mut e = self.effects.borrow_mut(); e.pipelines.push((desc.blend, desc.depth.unwrap())); e.pipelines.len() as u32
    }
    fn create_bind_group(&self, pipeline: &u32, _: u32, _: &[Binding<Self>]) -> u32 { *pipeline }
    fn create_encoder(&self, _: &str) {}
    fn encode_pass(&self, _: &mut (), _: &str, color: Option<&ColorAttachment<Self>>, depth: Option<&DepthAttachment<Self>>, commands: &[Command<Self>]) {
        let c = color.unwrap(); let d = depth.unwrap(); assert!(c.clear.is_none() && c.resolve.is_none()); assert!(d.clear.is_none() && d.store);
        let mut pipeline = 0;
        for command in commands { match command {
            Command::SetPipeline(p) => pipeline = **p,
            Command::SetBindGroup(0, b) => assert_eq!(pipeline, **b),
            Command::SetVertexBuffer(0, _) => {},
            Command::Draw { vertices, instances } => { assert_eq!(*vertices, 0..6); assert!(instances.start < instances.end && instances.end <= 1024); }
            _ => panic!("unexpected particle command"),
        }}
        let mut e = self.effects.borrow_mut(); e.passes += 1; e.commands = commands.len();
    }
    fn submit(&self, _: ()) { self.effects.borrow_mut().submits += 1; }
    fn wait_idle(&self) {}
    fn read_texture(&self, _: &u32) -> Vec<u8> { panic!("mock is not GPU evidence") }
    fn surface_format(&self, _: &()) -> TextureFormat { TextureFormat::Rgba8Unorm }
    fn resize_surface(&self, _: &mut (), _: u32, _: u32) {}
    fn acquire_frame(&self, _: &mut ()) -> Acquire<u32> { panic!("no window") }
    fn read_frame(&self, _: &u32) -> Option<Vec<u8>> { None }
    fn frame_view<'a>(&self, frame: &'a u32) -> &'a u32 { frame }
    fn present(&self, _: u32) { panic!("caller owns presentation") }
}
fn camera() -> Camera3D { Camera3D::orthographic([0.0,0.0,5.0], [0.0,0.0,0.0], 2.0) }
fn item() -> ParticleDraw {
    ParticleDraw { key: bytemuck::Zeroable::zeroed(), position: orr_view::Vec3::ZERO, size: 1.0, color: [1.0;4], blend: ParticleBlend::Alpha }
}
fn target() -> ParticleTarget<'static, Mock> { ParticleTarget { color: &1, depth: &2, size: (64,64), color_format: TextureFormat::Rgba8Unorm, depth_format: TextureFormat::Depth32Float, sample_count: 1, depth_size: (64,64), depth_sample_count: 1 } }
#[test]
fn layouts_blends_depth_and_load_contract_are_explicit() {
    assert_eq!(std::mem::size_of::<Globals>(),96); assert_eq!(std::mem::size_of::<Instance>(),32);
    let gpu = Mock::default(); let mut r = ParticleRenderer::new(gpu.clone(), TextureFormat::Rgba8Unorm).unwrap();
    let e=gpu.effects.borrow(); assert_eq!(e.buffers,vec![96,32768]); assert_eq!(e.pipelines.len(),2);
    for (_,d) in &e.pipelines { assert!(!d.write); assert_eq!(d.compare,Compare::LessEqual); }
    assert_eq!(e.pipelines[0].0,Blend::Alpha); assert_eq!(e.pipelines[1].0,Blend::Additive); drop(e);
    let p=r.prepare(&camera(),(64,64),&[item()]).unwrap(); r.upload_and_encode(&mut (),target(),&p).unwrap();
    let e=gpu.effects.borrow(); assert_eq!((e.writes,e.passes,e.submits),(2,1,0));
}
#[test]
fn unsupported_capabilities_reject_before_allocations() {
    for index in 0..6 {
        let mut gpu=Mock::default(); match index { 0=>gpu.color.blendable=false,1=>gpu.color.usages=TextureUsage(0),2=>gpu.depth.usages=TextureUsage(0),3=>gpu.samples=false,4=>gpu.color.max_dimension_2d=0,_=>gpu.depth.max_dimension_2d=0 }
        assert!(ParticleRenderer::new(gpu.clone(),TextureFormat::Rgba8Unorm).is_err()); assert!(gpu.effects.borrow().buffers.is_empty());
    }
    assert!(ParticleRenderer::new(Mock::default(),TextureFormat::Depth32Float).is_err());
}
#[test]
fn invalid_camera_and_viewport_never_write() {
    let gpu=Mock::default(); let r=ParticleRenderer::new(gpu.clone(),TextureFormat::Rgba8Unorm).unwrap();
    for size in [(0,1),(1,0),(8193,64)] { assert!(r.prepare(&camera(),size,&[item()]).is_err()); }
    for bad in [f32::NAN,f32::INFINITY,f32::NEG_INFINITY,f32::MAX] { for field in 0..3 { for axis in 0..3 {
        let mut c=camera(); match field {0=>c.eye[axis]=bad,1=>c.target[axis]=bad,_=>c.up[axis]=bad}; assert!(r.prepare(&c,(64,64),&[item()]).is_err());
    }}}
    let mut c=camera();c.eye=c.target;assert!(r.prepare(&c,(64,64),&[item()]).is_err());
    c=camera();c.up=c.forward();assert!(r.prepare(&c,(64,64),&[item()]).is_err());
    c=camera();c.projection=Projection::Perspective {fov_y:0.0,near:1.0,far:0.0};assert!(r.prepare(&c,(64,64),&[item()]).is_err());
    assert_eq!(gpu.effects.borrow().writes,0);
}
#[test]
fn malformed_last_item_and_all_float_fields_reject_atomically() {
    let gpu=Mock::default();let r=ParticleRenderer::new(gpu.clone(),TextureFormat::Rgba8Unorm).unwrap();
    for bad in [f32::NAN,f32::INFINITY,f32::NEG_INFINITY,f32::MAX] { for field in 0..8 {
        let mut i=item();match field {0=>i.position.x=bad,1=>i.position.y=bad,2=>i.position.z=bad,3=>i.size=bad,_=>i.color[field-4]=bad}
        assert!(r.prepare(&camera(),(64,64),&[item(),i]).is_err());
    }}
    assert!(matches!(r.prepare(&camera(),(64,64),&vec![item();1025]),Err(ParticleRenderError::Capacity)));
    assert_eq!(gpu.effects.borrow().writes,0);
}
#[test]
fn depth_sort_uses_signed_forward_distance_and_stable_ties() {
    let r=ParticleRenderer::new(Mock::default(),TextureFormat::Rgba8Unorm).unwrap();
    let mut a=item();a.position=orr_view::Vec3::new(100.0,0.0,4.0);a.color[0]=0.1;
    let mut b=item();b.position.z=0.0;b.color[0]=0.2;b.blend=ParticleBlend::Additive;
    let mut c=b;c.color[0]=0.3;c.blend=ParticleBlend::Alpha;
    let p=r.prepare(&camera(),(64,64),&[a,b,c]).unwrap();
    assert_eq!(p.instances.iter().map(|i|i.color[0]).collect::<Vec<_>>(),vec![0.2,0.3,0.1]);
    assert_eq!(p.draw_count(),2);
}
#[test]
fn alternating_blends_have_bounded_exact_command_count() {
    let gpu=Mock::default();let mut r=ParticleRenderer::new(gpu.clone(),TextureFormat::Rgba8Unorm).unwrap();
    let items:Vec<_>=(0..1024).map(|n|ParticleDraw { blend:if n%2==0 {ParticleBlend::Alpha}else{ParticleBlend::Additive},..item() }).collect();
    let p=r.prepare(&camera(),(64,64),&items).unwrap();assert_eq!(p.draw_count(),1024);assert_eq!(p.particle_count(),1024);
    r.upload_and_encode(&mut (),target(),&p).unwrap();assert_eq!(gpu.effects.borrow().commands,3073);
}
#[test]
fn preparation_owns_bytes_and_empty_packet_has_no_gpu_effects() {
    let gpu=Mock::default();let mut r=ParticleRenderer::new(gpu.clone(),TextureFormat::Rgba8Unorm).unwrap();
    let first=r.prepare(&camera(),(64,64),&[item()]).unwrap();let second=r.prepare(&camera(),(64,64),&[]).unwrap();
    assert_eq!(first.particle_count(),1);r.upload_and_encode(&mut (),target(),&second).unwrap();assert_eq!(gpu.effects.borrow().writes,0);
    r.upload_and_encode(&mut (),target(),&first).unwrap();assert_eq!(gpu.effects.borrow().writes,2);
}
#[test]
fn target_mismatch_and_cross_format_packet_reject_before_writes() {
    let gpu=Mock::default();let mut r=ParticleRenderer::new(gpu.clone(),TextureFormat::Rgba8Unorm).unwrap();let p=r.prepare(&camera(),(64,64),&[item()]).unwrap();
    for field in 0..6 {let mut t=target();match field {0=>t.size=(32,64),1=>t.color_format=TextureFormat::Rgba16Float,2=>t.depth_format=TextureFormat::Rgba8Unorm,3=>t.sample_count=4,4=>t.depth_size=(32,64),_=>t.depth_sample_count=4};assert!(r.upload_and_encode(&mut (),t,&p).is_err());}
    let other=ParticleRenderer::new(Mock::default(),TextureFormat::Rgba16Float).unwrap();let other=other.prepare(&camera(),(64,64),&[item()]).unwrap();
    assert!(r.upload_and_encode(&mut (),target(),&other).is_err());assert_eq!(gpu.effects.borrow().writes,0);
}
#[test]
fn original_presets_fit_pool_contract() {
    for d in [crate::particle_presets::landing_dust(orr_view::Vec3::ZERO,1),crate::particle_presets::pickup_sparks(orr_view::Vec3::ZERO,2)] {d.validate().unwrap();assert!(d.burst<=16);}
}
