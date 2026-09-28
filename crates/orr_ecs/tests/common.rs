use bytemuck::{Pod, Zeroable};
use orr_ecs::{ComponentRegistry, ComponentRegistryBuilder};
use std::sync::Arc;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct Pos {
    pub x: i64,
    pub y: i64,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct Vel {
    pub x: i64,
    pub y: i64,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct Health {
    pub hp: i32,
    pub max: i32,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct Tag {
    pub value: u32,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct GameConfig {
    pub gravity: i64,
    pub max_players: u64,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Pod, Zeroable)]
pub struct Score {
    pub value: i64,
}

#[allow(dead_code)]
pub fn test_registry() -> Arc<ComponentRegistry> {
    let mut b = ComponentRegistryBuilder::new();
    b.register_component::<Pos>("Pos");
    b.register_component::<Vel>("Vel");
    b.register_component::<Health>("Health");
    b.register_component::<Tag>("Tag");
    b.register_singleton::<GameConfig>("GameConfig");
    b.register_list::<Score>("Score");
    b.build()
}
