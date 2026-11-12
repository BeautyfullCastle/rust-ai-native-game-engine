use super::*;
use orr_bridge::FrameView;
use crate::room_game::{RoomActor, PLAYER, RoomConfig, RoomEscapeV1};
use orr_sim::{PlayerSlot, TickInputs};
fn doc() -> Document {
    Document { schema:3, follow_player:Some([1.0,2.0,-1.0]),
        target_bounds:Some(TargetBounds {min:[0.0,1.5,-0.5],max:[1.1,1.5,0.5]}),
        ..Document::readable_default() }
}
#[test]
fn limits_strict_schema_roundtrip_and_legacy_omission() {
    let good=doc();let bytes=good.to_bytes().unwrap();assert_eq!(Document::parse(&bytes).unwrap(),good);
    for schema in [1,2] {let mut bad=good.clone();bad.schema=schema;assert!(bad.validate().is_err());}
    let mut legacy=good.clone();legacy.schema=2;legacy.target_bounds=None;
    let bytes=legacy.to_bytes().unwrap();assert!(!String::from_utf8(bytes.clone()).unwrap().contains("target_bounds"));assert_eq!(Document::parse(&bytes).unwrap().to_bytes().unwrap(),bytes);
    for remove in ["follow_player","target_bounds"] {let mut bad=serde_json::to_value(&good).unwrap();bad.as_object_mut().unwrap().remove(remove);assert!(Document::parse(&serde_json::to_vec(&bad).unwrap()).is_err());}
    for badbounds in [serde_json::json!(null),serde_json::json!([[0,0,0],[1,1,1]]),serde_json::json!({"min":[0,0,0]}),serde_json::json!({"min":[0,0,0],"max":[1,1,1],"extra":0}),serde_json::json!({"min":[0,0],"max":[1,1,1]})] {
        let mut bad=serde_json::to_value(&good).unwrap();bad["target_bounds"]=badbounds;assert!(Document::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    let text=String::from_utf8(good.to_bytes().unwrap()).unwrap();
    assert!(Document::parse(text.replace("\"min\":", "\"min\":[0,0,0],\"min\":").as_bytes()).is_err());
    assert!(Document::parse(text.replace("\"target_bounds\":", "\"target_bounds\":null,\"target_bounds\":").as_bytes()).is_err());
}
#[test]
fn invalid_limits_reject_without_mutating_frame_or_session() {
    let sim=orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig,60,42);let frame=sim.frame().to_bytes();
    for value in [f32::NAN,f32::INFINITY,-65.0,65.0] {
        for is_min in [true,false] {let mut bad=doc();let b=bad.target_bounds.as_mut().unwrap();if is_min {b.min[0]=value;} else {b.max[0]=value;}
            let orbit=bad.orbit();assert!(bad.to_bytes().is_err());assert!(bad.camera_for_frame(&orbit,(800,600),FrameView::of(sim.frame())).is_err());assert_eq!(orbit,bad.orbit());}
    }
    let mut bad=doc();bad.target_bounds.as_mut().unwrap().min[0]=2.0;assert!(bad.validate().is_err());
    assert_eq!(sim.frame().to_bytes(),frame);
}
#[test]
fn real_movement_and_session_pan_clamp_after_saved_offset_with_axis_lock() {
    let d=doc();let orbit=d.orbit();let mut sim=orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig,60,42);
    let initial=d.camera_for_frame(&orbit,(800,600),FrameView::of(sim.frame())).unwrap();assert_eq!(initial.target,[1.0,1.5,-0.5]);
    for _ in 0..20 {let mut input=TickInputs::new(sim.tick()+1,1);input.set_input(PlayerSlot(0),orr_games::room_escape_game::RoomInput {move_x:1,move_z:-1,..Default::default()});sim.step(&input);}
    let before=sim.frame().to_bytes();let entity=sim.frame().entities().find(|&e|sim.frame().get::<RoomActor>(e).is_some_and(|a|a.kind==PLAYER)).unwrap();let body=sim.frame().get::<orr_physics3d::Body>(entity).unwrap();assert!(body.pos.x.to_f32()>0.1);
    let camera=d.camera_for_frame(&orbit,(800,600),FrameView::of(sim.frame())).unwrap();assert_eq!(camera.target,[1.1,1.5,-0.5]);
    let mut panned=orbit;panned.target=[-64.0,-64.0,64.0];let p=d.camera_for_frame(&panned,(800,600),FrameView::of(sim.frame())).unwrap();assert_eq!(p.target,[0.0,1.5,0.5]);
    assert_eq!(sim.frame().to_bytes(),before);assert_eq!(orbit,d.orbit());
    // Edit and capture remain authored fixed framing even outside the limits.
    assert_eq!(d.camera(&orbit,(800,600)).unwrap().target,d.target);
}
#[test]
fn limit_endpoints_and_extreme_aspects_stay_finite_and_seek_is_stateless() {
    let mut d=doc();let mut sim=orr_sim::Simulation::<RoomEscapeV1>::new(RoomConfig,60,42);let first=sim.frame().clone();
    for _ in 0..8 {sim.step(&TickInputs::new(sim.tick()+1,1));}
    for bounds in [TargetBounds{min:[-64.0;3],max:[64.0;3]},TargetBounds{min:[64.0;3],max:[64.0;3]},TargetBounds{min:[-64.0;3],max:[-64.0;3]}] {
        d.target_bounds=Some(bounds);
        for projection in [Projection::Perspective{fov_y_degrees:55.0},Projection::Orthographic{half_height:8.0}] {d.projection=projection;let orbit=d.orbit();
            for size in [(1,16384),(16384,1),(800,600)] {let c=d.camera_for_frame(&orbit,size,FrameView::of(sim.frame())).unwrap();assert!(c.view_proj(size.0 as f32/size.1 as f32).0.iter().flatten().all(|v|v.is_finite()));let (origin,ray)=c.screen_ray([0.0,0.0],size);assert!(origin.iter().chain(ray.iter()).all(|v|v.is_finite()));}
            assert_eq!(d.camera_for_frame(&orbit,(800,600),FrameView::of(&first)).unwrap(),d.camera_for_frame(&orbit,(800,600),FrameView::of(sim.frame())).unwrap());
        }
    }
}
