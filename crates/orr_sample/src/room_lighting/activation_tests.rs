use super::*;
use crate::room_game::{RoomConfig,RoomEscapeV1,RoomInput,RoomRun,RoomActor,PLAYER,INTERACT};
use orr_bridge::FrameView;
use orr_sim::{Simulation,TickInputs,PlayerSlot};
fn conditional(a:Activation)->Document {Document {activation:Some(a),..Document::enabled_default()}}
fn sim()->Simulation<RoomEscapeV1> {Simulation::new(RoomConfig,60,42)}
fn step(sim:&mut Simulation<RoomEscapeV1>,input:RoomInput) {let mut tick=TickInputs::new(sim.tick()+1,1);tick.set_input(PlayerSlot(0),input);sim.step(&tick);}
#[test]
fn conditional_codec_is_strict_and_preserves_legacy_bytes() {
    for follow in [None,Some([1.0,2.0,-1.0])] {for activation in [Activation::KeyCollected,Activation::Escaped] {let d=Document {follow_player:follow,..conditional(activation)};let bytes=d.to_bytes().unwrap();assert_eq!(Document::parse(&bytes).unwrap(),d);
        let good:serde_json::Value=serde_json::from_slice(&bytes).unwrap();
        for value in [serde_json::json!(null),serde_json::json!({"key_collected":null}),serde_json::json!(["escaped"]),serde_json::json!("always"),serde_json::json!("won")] {let mut bad=good.clone();bad["activation"]=value;assert!(Document::parse(&serde_json::to_vec(&bad).unwrap()).is_err());}
        for version in [1,2,4] {let mut bad=good.clone();bad["version"]=serde_json::json!(version);assert!(Document::parse(&serde_json::to_vec(&bad).unwrap()).is_err());}
        let mut missing=good.clone();missing.as_object_mut().unwrap().remove("activation");assert!(Document::parse(&serde_json::to_vec(&missing).unwrap()).is_err());
        let text=String::from_utf8(bytes).unwrap();assert!(Document::parse(text.replace("\"activation\":","\"activation\":\"escaped\",\"activation\":").as_bytes()).is_err());
    }}
    for follow in [None,Some([1.0,2.0,-1.0])] {let d=Document {follow_player:follow,..Document::enabled_default()};let bytes=d.to_bytes().unwrap();assert!(!String::from_utf8(bytes.clone()).unwrap().contains("activation"));assert_eq!(Document::parse(&bytes).unwrap().to_bytes().unwrap(),bytes);}
}
#[test]
fn actual_pickup_and_victory_gate_light_without_frame_writes_and_seek_resets() {
    let key=conditional(Activation::KeyCollected);let won=conditional(Activation::Escaped);let mut sim=sim();let initial=sim.frame().clone();
    assert_eq!(key.settings_for_frame(FrameView::of(&initial)).unwrap().point_light,None);assert_eq!(won.settings_for_frame(FrameView::of(&initial)).unwrap().point_light,None);
    let mut missed=Simulation::<RoomEscapeV1>::new(RoomConfig,60,42);for _ in 0..40 {step(&mut missed,RoomInput {move_x:-1,..Default::default()});}step(&mut missed,RoomInput {buttons:INTERACT,..Default::default()});assert_eq!(missed.frame().singleton::<RoomRun>().key_collected,0);assert_eq!(key.settings_for_frame(FrameView::of(missed.frame())).unwrap().point_light,None);
    step(&mut sim,RoomInput {buttons:INTERACT,..Default::default()});assert_eq!(sim.frame().singleton::<RoomRun>().key_collected,1);assert_eq!(sim.frame().singleton::<RoomRun>().won,0);
    let before=sim.frame().to_bytes();assert_eq!(key.settings_for_frame(FrameView::of(sim.frame())).unwrap(),key.settings());assert_eq!(won.settings_for_frame(FrameView::of(sim.frame())).unwrap().point_light,None);assert_eq!(sim.frame().to_bytes(),before);
    for _ in 0..40 {step(&mut sim,RoomInput {move_x:1,..Default::default()});}step(&mut sim,RoomInput {buttons:INTERACT,..Default::default()});assert_eq!(sim.frame().singleton::<RoomRun>().won,1);
    let before=sim.frame().to_bytes();assert_eq!(key.settings_for_frame(FrameView::of(sim.frame())).unwrap(),key.settings());assert_eq!(won.settings_for_frame(FrameView::of(sim.frame())).unwrap(),won.settings());assert_eq!(sim.frame().to_bytes(),before);
    assert_eq!(won.settings_for_frame(FrameView::of(&initial)).unwrap().point_light,None);assert_eq!(key.settings_for_frame(FrameView::of(&initial)).unwrap().point_light,None);
}
#[test]
fn conditional_carried_light_still_validates_inactive_body_and_follows_after_pickup() {
    let d=Document {follow_player:Some([1.0,2.0,-1.0]),..conditional(Activation::KeyCollected)};let mut sim=sim();assert_eq!(d.settings_for_frame(FrameView::of(sim.frame())).unwrap().point_light,None);
    let mut missing=sim.frame().clone();let player=missing.entities().find(|&e|missing.get::<RoomActor>(e).is_some_and(|a|a.kind==PLAYER)).unwrap();missing.remove::<orr_physics3d::Body>(player);assert!(d.settings_for_frame(FrameView::of(&missing)).is_err());
    step(&mut sim,RoomInput {move_x:1,buttons:INTERACT,..Default::default()});assert_eq!(sim.frame().singleton::<RoomRun>().key_collected,1);let body=sim.frame().get::<orr_physics3d::Body>(player).unwrap();assert_eq!(d.settings_for_frame(FrameView::of(sim.frame())).unwrap().point_light.unwrap().position,[body.pos.x.to_f32()+1.0,body.pos.y.to_f32()+2.0,body.pos.z.to_f32()-1.0]);
    assert_eq!(d.settings().point_light.unwrap().position,[0.0,4.0,0.0]);
}
#[test]
fn disabled_or_invalid_conditional_light_cannot_be_saved_or_hidden_by_inactive_state() {
    let sim=sim();let mut d=conditional(Activation::Escaped);d.point_light=None;assert!(d.validate().is_err());assert!(d.to_bytes().is_err());assert!(d.settings_for_frame(FrameView::of(sim.frame())).is_err());
    let mut d=conditional(Activation::Escaped);d.point_light.as_mut().unwrap().intensity=f32::NAN;assert!(d.settings_for_frame(FrameView::of(sim.frame())).is_err());
    let empty=orr_ecs::Frame::new(orr_ecs::ComponentRegistryBuilder::new().build());let d=Document::enabled_default();assert_eq!(d.settings_for_frame(FrameView::of(&empty)).unwrap(),d.settings());
}
