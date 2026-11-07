use super::*;
use crate::room_game::{RoomActor,RoomConfig,RoomEscapeV1,RoomInput,PLAYER,KEY};
use orr_bridge::FrameView;
use orr_sim::{Simulation,TickInputs,PlayerSlot};
fn doc()->Document {Document {follow_player:Some([1.0,2.0,-1.0]),..Document::enabled_default()}}
fn sim()->Simulation<RoomEscapeV1> {Simulation::new(RoomConfig,60,42)}
fn player(frame:&orr_ecs::Frame)->orr_ecs::Entity {frame.entities().find(|&e|frame.get::<RoomActor>(e).is_some_and(|a|a.kind==PLAYER)).unwrap()}
#[test]
fn strict_carried_document_and_legacy_renderer_bytes() {
    let d=doc();let bytes=d.to_bytes().unwrap();assert_eq!(Document::parse(&bytes).unwrap(),d);
    let legacy=Document::enabled_default();assert_eq!(legacy.to_bytes().unwrap(),legacy.settings().to_bytes().unwrap());assert!(!String::from_utf8(legacy.to_bytes().unwrap()).unwrap().contains("follow_player"));
    let good=serde_json::to_value(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()).unwrap();
    for offset in [serde_json::json!(null),serde_json::json!([1,2]),serde_json::json!([1,2,3,4]),serde_json::json!([33,0,0]),serde_json::json!({"x":0})] {
        let mut bad=good.clone();bad["follow_player"]=offset;assert!(Document::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    for version in [0,1,3] {let mut bad=good.clone();bad["version"]=serde_json::json!(version);assert!(Document::parse(&serde_json::to_vec(&bad).unwrap()).is_err());}
    let mut bad=good.clone();bad.as_object_mut().unwrap().remove("follow_player");assert!(Document::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
    let mut bad=good;bad["point_light"]=serde_json::Value::Null;assert!(Document::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
    let text=String::from_utf8(bytes).unwrap();assert!(Document::parse(text.replace("\"follow_player\":","\"follow_player\":[0,0,0],\"follow_player\":").as_bytes()).is_err());
}
#[test]
fn actual_movement_resolves_same_frame_preserving_authored_light_and_frame() {
    let d=doc();let mut sim=sim();let first=sim.frame().clone();
    let initial=d.settings_for_frame(FrameView::of(&first)).unwrap();assert_eq!(initial.point_light.unwrap().position,[1.0,2.5,-1.0]);
    for _ in 0..12 {let mut input=TickInputs::new(sim.tick()+1,1);input.set_input(PlayerSlot(0),RoomInput {move_x:1,move_z:-1,..Default::default()});sim.step(&input);}
    let bytes=sim.frame().to_bytes();let body=sim.frame().get::<orr_physics3d::Body>(player(sim.frame())).unwrap();let pos=[body.pos.x.to_f32()+1.0,body.pos.y.to_f32()+2.0,body.pos.z.to_f32()-1.0];
    let resolved=d.settings_for_frame(FrameView::of(sim.frame())).unwrap();assert_eq!(resolved.point_light.unwrap().position,pos);assert_ne!(pos,initial.point_light.unwrap().position);
    let mut expected=d.settings();expected.point_light.as_mut().unwrap().position=pos;assert_eq!(resolved,expected);assert_eq!(d.settings().point_light.unwrap().position,[0.0,4.0,0.0]);
    assert_eq!(d.settings_for_frame(FrameView::of(&first)).unwrap(),initial);assert_eq!(d.settings_for_frame(FrameView::of(sim.frame())).unwrap(),resolved);assert_eq!(sim.frame().to_bytes(),bytes);
}
#[test]
fn missing_duplicate_body_or_out_of_range_player_rejects_without_mutation() {
    let d=doc();let sim=sim();let base=sim.frame();let mut missing=base.clone();missing.despawn(player(base));assert!(d.settings_for_frame(FrameView::of(&missing)).is_err());
    let mut duplicate=base.clone();let key=duplicate.entities().find(|&e|duplicate.get::<RoomActor>(e).is_some_and(|a|a.kind==KEY)).unwrap();duplicate.get_mut::<RoomActor>(key).unwrap().kind=PLAYER;assert!(d.settings_for_frame(FrameView::of(&duplicate)).is_err());
    let mut nobody=base.clone();nobody.remove::<orr_physics3d::Body>(player(base));assert!(d.settings_for_frame(FrameView::of(&nobody)).is_err());
    let mut large=base.clone();large.get_mut::<orr_physics3d::Body>(player(base)).unwrap().pos.x=orr_fp::FP::from_int(33);let bytes=large.to_bytes();assert!(d.settings_for_frame(FrameView::of(&large)).is_err());assert_eq!(large.to_bytes(),bytes);
}
#[test]
fn offset_limits_and_disabled_or_invalid_policy_fail_closed() {
    let frame=orr_ecs::Frame::new(orr_ecs::ComponentRegistryBuilder::new().build());let fixed=Document::enabled_default();assert_eq!(fixed.settings_for_frame(FrameView::of(&frame)).unwrap(),fixed.settings());
    let sim=sim();
    for value in [f32::NAN,f32::INFINITY,33.0,-33.0] {let mut d=doc();d.follow_player.as_mut().unwrap()[0]=value;assert!(d.to_bytes().is_err());assert!(d.settings_for_frame(FrameView::of(sim.frame())).is_err());}
    let invalid=Document {point_light:None,follow_player:Some([0.0;3]),activation:None,environment:None};assert!(invalid.validate().is_err());assert!(invalid.settings_for_frame(FrameView::of(sim.frame())).is_err());
    for offset in [[-32.0;3],[32.0;3]] {let mut d=doc();d.follow_player=Some(offset);let settings=d.settings_for_frame(FrameView::of(sim.frame())).unwrap();settings.validate().unwrap();assert_eq!(settings.point_light.unwrap().position,[offset[0],offset[1]+0.5,offset[2]]);}
}
