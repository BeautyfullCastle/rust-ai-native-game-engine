//! Test types shared by the orr_reflect integration tests.
#![allow(dead_code)]

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistry, ComponentRegistryBuilder, Entity, Frame};
use orr_fp::{FPVec2, FP, FP32};
use orr_reflect::{Reflect, TaggedDesc, TypeDesc, TypeRegistry, Value, VariantDesc, ViewField};

/// Position and heading.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable, Reflect)]
pub struct Transform {
    /// World position.
    #[reflect(range = "-1000..=1000")]
    pub pos: FPVec2,
    /// Heading in degrees.
    pub rot: FP,
}

/// Hit points.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable, Reflect)]
#[reflect(default)]
pub struct Health {
    pub current: i32,
    #[reflect(range = "1..=1000")]
    pub max: i32,
    #[reflect(bool)]
    pub alive: u32,
    #[reflect(skip)]
    pub cache: u32,
}

impl Default for Health {
    fn default() -> Self {
        Health { current: 100, max: 100, alive: 1, cache: 0 }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable, Reflect)]
pub struct Follow {
    pub target: Entity,
    #[reflect(enumeration = "idle=0,chase=1,flee=2")]
    pub mode: u32,
    #[reflect(flags = "fast=1,armored=2,flying=4")]
    pub traits: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable, Reflect)]
pub struct Stats {
    pub scores: [u32; 4],
    pub gravity: FPVec2,
    pub weight: FP32,
    #[reflect(skip)]
    pub pad: u32,
}

/// A shape with a friendly tagged view over raw bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Blob {
    pub kind: u32,
    pub count: u32,
    pub a: FP,
    pub pts: [FPVec2; 4],
}

fn blob_read(b: &[u8]) -> Value {
    let s: Blob = bytemuck::pod_read_unaligned(b);
    match s.kind {
        0 => Value::Variant("dot".into(), vec![("radius".into(), Value::Fixed(s.a))]),
        1 => Value::Variant("box".into(), vec![("half".into(), Value::Vec2(s.pts[0]))]),
        _ => Value::Variant(
            "poly".into(),
            vec![("pts".into(), Value::Array(s.pts[..s.count as usize].iter().map(|p| Value::Vec2(*p)).collect()))],
        ),
    }
}

fn blob_write(b: &mut [u8], v: &Value) -> Result<(), String> {
    let Value::Variant(name, f) = v else { return Err("not a variant".into()) };
    let mut s = Blob::zeroed();
    match name.as_str() {
        "dot" => {
            s.kind = 0;
            if let Some((_, Value::Fixed(r))) = f.first() {
                s.a = *r;
            }
        }
        "box" => {
            s.kind = 1;
            if let Some((_, Value::Vec2(h))) = f.first() {
                s.pts[0] = *h;
            }
        }
        _ => {
            s.kind = 2;
            let Some((_, Value::Array(pts))) = f.first() else { return Err("no points".into()) };
            // Same kind of rule as a physics polygon: the points must turn left.
            let p: Vec<FPVec2> = pts.iter().map(|x| if let Value::Vec2(v) = x { *v } else { FPVec2::ZERO }).collect();
            for i in 0..p.len() {
                let e1 = p[(i + 1) % p.len()] - p[i];
                let e2 = p[(i + 2) % p.len()] - p[(i + 1) % p.len()];
                if e1.perp_dot(e2) <= FP::ZERO {
                    return Err("pts: polygon must be convex and counter-clockwise".into());
                }
            }
            s.count = p.len() as u32;
            s.pts[..p.len()].copy_from_slice(&p);
        }
    }
    b.copy_from_slice(bytemuck::bytes_of(&s));
    Ok(())
}

impl Reflect for Blob {
    fn describe() -> TypeDesc {
        let pt = TypeDesc::vec2().with_range_str("-100..=100");
        TypeDesc::tagged(TaggedDesc {
            size: core::mem::size_of::<Blob>(),
            variants: vec![
                VariantDesc {
                    name: "dot".into(),
                    doc: "A circle.".into(),
                    fields: vec![ViewField::new("radius", TypeDesc::fixed().with_range_str("0.05..=100"), "Radius.")],
                    default: vec![("radius".into(), Value::Fixed(FP::ONE))],
                },
                VariantDesc {
                    name: "box".into(),
                    doc: String::new(),
                    fields: vec![ViewField::new("half", pt.clone(), "Half extents.")],
                    default: vec![("half".into(), Value::Vec2(FPVec2::new(FP::ONE, FP::ONE)))],
                },
                VariantDesc {
                    name: "poly".into(),
                    doc: String::new(),
                    fields: vec![ViewField::new("pts", TypeDesc::list(pt, 3, 4), "Corners.")],
                    default: vec![(
                        "pts".into(),
                        Value::Array(vec![
                            Value::Vec2(FPVec2::new(FP::ZERO, FP::ZERO)),
                            Value::Vec2(FPVec2::new(FP::ONE, FP::ZERO)),
                            Value::Vec2(FPVec2::new(FP::ZERO, FP::ONE)),
                        ]),
                    )],
                },
            ],
            read: blob_read,
            write: blob_write,
        })
        .with_doc("A blob.")
    }
    fn default_value() -> Self {
        Blob { kind: 0, count: 0, a: FP::ONE, pts: [FPVec2::ZERO; 4] }
    }
}

pub fn types() -> TypeRegistry {
    let mut t = TypeRegistry::new();
    t.register_component::<Transform>("Transform");
    t.register_component::<Health>("Health");
    t.register_component::<Follow>("Follow");
    t.register_component::<Blob>("Blob");
    t.register_singleton::<Stats>("Stats");
    t
}

pub fn ecs_registry() -> Arc<ComponentRegistry> {
    let mut b = ComponentRegistryBuilder::new();
    b.register_component::<Transform>("Transform");
    b.register_component::<Health>("Health");
    b.register_component::<Follow>("Follow");
    b.register_component::<Blob>("Blob");
    b.register_singleton::<Stats>("Stats");
    b.build()
}

pub fn frame() -> Frame {
    Frame::new(ecs_registry())
}

pub const SAMPLE: &str = r#"# A test scene.
schema: orr.scene/1
singletons:
  Stats: { scores: [1, 2, 3, 4], gravity: [0, -9.8125], weight: 1.5 }
entities:
  # The hero.
  e_00000001:
    name: Hero
    # Where it stands.
    Blob: { kind: box, half: [0.5, 0.25] }
    Follow: { target: e_00000002, mode: chase, traits: [fast, flying] }
    Health: { current: 80, max: 100, alive: true }
    Transform: { pos: [12.5, -4], rot: 90 }
  e_00000002:
    name: "no"
    Follow: { target: null, mode: idle, traits: [] }
    Transform: { pos: [-3.25, 7], rot: 0 }
  e_00000003: {}
"#;
