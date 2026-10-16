#![cfg(feature = "particles")]
//! Device-required acceptance. Explicitly ignored by CPU lanes, never skip-success.
use orr_render::{Camera3D, particle_renderer::{ParticleRenderer, ParticleTarget}};
use orr_rhi::{Rhi, Wgpu, WgpuOptions, TextureFormat, TextureDesc, TextureUsage, ColorAttachment, DepthAttachment};
use orr_view::{Vec3, particles::{ParticleDraw, ParticleBlend}};
#[test]
#[ignore = "requires separately admitted GPU lane"]
fn particle_shader_depth_blend_order_camera_and_rejection_readback() {
    let gpu = Wgpu::headless(WgpuOptions::default()).expect("required particle GPU adapter unavailable");
    eprintln!("particle adapter={} software={}", gpu.adapter_name(), gpu.is_software());
    let color = gpu.create_texture(&TextureDesc { label: "particle acceptance color", width:64,height:64,format:TextureFormat::Rgba8Unorm,
        usage:TextureUsage::RENDER_ATTACHMENT|TextureUsage::COPY_SRC,sample_count:1,view_formats:&[] });
    let depth = gpu.create_texture(&TextureDesc { label: "particle acceptance depth", width:64,height:64,format:TextureFormat::Depth32Float,
        usage:TextureUsage::RENDER_ATTACHMENT,sample_count:1,view_formats:&[] });
    let color_view=gpu.create_texture_view(&color,None);let depth_view=gpu.create_texture_view(&depth,None);
    let mut renderer=ParticleRenderer::new(gpu.clone(),TextureFormat::Rgba8Unorm).unwrap();
    let camera=Camera3D::orthographic([0.0,0.0,5.0],[0.0;3],2.0);
    let item=ParticleDraw { key:bytemuck::Zeroable::zeroed(),position:Vec3::ZERO,size:1.0,color:[1.0,0.0,0.0,0.5],blend:ParticleBlend::Alpha };
    let clear = |z| {
        let mut encoder=gpu.create_encoder("particle acceptance clear");
        gpu.encode_pass(&mut encoder,"clear",Some(&ColorAttachment {view:&color_view,clear:Some([0.0,0.0,0.0,0.25]),resolve:None}),
            Some(&DepthAttachment {view:&depth_view,clear:Some(z),store:true}),&[]);gpu.submit(encoder);
    };
    let draw = |renderer:&mut ParticleRenderer<Wgpu>, camera:&Camera3D, items:&[ParticleDraw]| {
        let packet=renderer.prepare(camera,(64,64),items).unwrap();let mut encoder=gpu.create_encoder("particle acceptance draw");
        renderer.upload_and_encode(&mut encoder,ParticleTarget {color:&color_view,depth:&depth_view,size:(64,64),color_format:TextureFormat::Rgba8Unorm,
            depth_format:TextureFormat::Depth32Float,sample_count:1,depth_size:(64,64),depth_sample_count:1},&packet).unwrap();gpu.submit(encoder);
    };
    let center = |pixels:&[u8]| -> [u8;4] { pixels[(32*64+32)*4..(32*64+32)*4+4].try_into().unwrap() };
    let close = |actual:[u8;4], expected:[u8;4]| { for (a,b) in actual.into_iter().zip(expected) {assert!(a.abs_diff(b)<=3,"actual {actual:?}, expected {expected:?}");} };
    clear(0.0);let occluded=gpu.read_texture(&color);draw(&mut renderer,&camera,&[item]);assert_eq!(gpu.read_texture(&color),occluded,"opaque depth blocks particles");
    clear(1.0);draw(&mut renderer,&camera,&[item]);close(center(&gpu.read_texture(&color)),[128,0,0,159]);
    let blue=ParticleDraw {color:[0.0,0.0,1.0,0.5],blend:ParticleBlend::Additive,..item};
    draw(&mut renderer,&camera,&[blue]);close(center(&gpu.read_texture(&color)),[128,0,128,159]);
    let far=ParticleDraw {color:[0.0,1.0,0.0,0.5],blend:ParticleBlend::Additive,..item};
    let near=ParticleDraw {position:Vec3::new(0.0,0.0,1.0),..item};
    clear(1.0);draw(&mut renderer,&camera,&[near,far]);close(center(&gpu.read_texture(&color)),[128,64,0,159]);
    clear(1.0);draw(&mut renderer,&camera,&[near]);draw(&mut renderer,&camera,&[far]);close(center(&gpu.read_texture(&color)),[128,128,0,159]);
    let rotated=Camera3D::orthographic([5.0,0.0,0.0],[0.0;3],2.0);clear(1.0);draw(&mut renderer,&rotated,&[item]);
    let before=gpu.read_texture(&color);close(center(&before),[128,0,0,159]);
    assert!(before.chunks_exact(4).filter(|p|p[0]>0).count()>100,"billboard retains area when camera rotates");
    let invalid=ParticleDraw {size:f32::NAN,..item};assert!(renderer.prepare(&rotated,(64,64),&[item,invalid]).is_err());assert_eq!(before,gpu.read_texture(&color));
}
